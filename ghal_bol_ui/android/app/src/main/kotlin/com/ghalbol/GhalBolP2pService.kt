package com.ghalbol

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import java.io.File

/**
 * Foreground service in process `:p2p` — hosts libp2p + JSON-RPC over a Unix socket.
 */
class GhalBolP2pService : Service() {

    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var multicastLock: WifiManager.MulticastLock? = null
    private var daemonThread: Thread? = null
    private var connectivityCallback: ConnectivityManager.NetworkCallback? = null
    private var wifiConnectivityCallback: ConnectivityManager.NetworkCallback? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private var lastDefaultNetwork: Network? = null
    private var lastTransportsKey: String = ""
    private val networkNotifyRunnable = Runnable {
        try {
            acquireMulticastLock()
            P2pDaemonNative.notifyNetworkChange()
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "notifyNetworkChange: ${e.message}")
        }
    }
    private val restartRunnable = Runnable {
        if (userRequestedStop) return@Runnable
        try {
            ContextCompat.startForegroundService(
                applicationContext,
                Intent(applicationContext, GhalBolP2pService::class.java),
            )
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "p2p service restart: ${e.message}")
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun configureNativeDataDir() {
        val dir = NativeStorage.dataRoot(this).absolutePath
        try {
            P2pDaemonNative.initTls(applicationContext)
            P2pDaemonNative.initAudio(applicationContext)
            P2pDaemonNative.configureDataDirectory(dir)
            android.util.Log.i("GhalBol", "p2p data dir=$dir")
        } catch (e: Throwable) {
            android.util.Log.e("GhalBol", "configureDataDirectory: ${e.message}")
        }
    }

    private fun acquireWakeLock() {
        releaseWakeLock()
        try {
            val pm = getSystemService(POWER_SERVICE) as? PowerManager ?: return
            wakeLock =
                pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "ghalbol:p2p_listener").apply {
                    setReferenceCounted(false)
                    acquire(6 * 60 * 60 * 1000L)
                }
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "wake lock (p2p): ${e.message}")
        }
    }

    private fun releaseWakeLock() {
        try {
            wakeLock?.let {
                if (it.isHeld) it.release()
            }
        } catch (_: Throwable) {
        }
        wakeLock = null
    }

    @Suppress("DEPRECATION")
    private fun acquireWifiLock() {
        releaseWifiLock()
        try {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager ?: return
            val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                WifiManager.WIFI_MODE_FULL_LOW_LATENCY
            } else {
                WifiManager.WIFI_MODE_FULL_HIGH_PERF
            }
            wifiLock = wm.createWifiLock(mode, "ghal_bol_p2p_wifi").apply {
                setReferenceCounted(false)
                acquire()
            }
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "wifi lock (p2p): ${e.message}")
        }
    }

    private fun releaseWifiLock() {
        try {
            wifiLock?.let { if (it.isHeld) it.release() }
        } catch (_: Throwable) {}
        wifiLock = null
    }

    private fun acquireMulticastLock() {
        if (multicastLock?.isHeld == true) return
        releaseMulticastLock()
        try {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager ?: return
            multicastLock =
                wm.createMulticastLock("ghal_bol_mdns_p2p").apply {
                    setReferenceCounted(false)
                    acquire()
                }
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "multicast lock (p2p): ${e.message}")
        }
    }

    private fun releaseMulticastLock() {
        try {
            multicastLock?.let {
                if (it.isHeld) it.release()
            }
        } catch (_: Throwable) {
        }
        multicastLock = null
    }

    private fun socketFile(): File {
        val dir = File(filesDir, "ghalbol")
        if (!dir.exists()) dir.mkdirs()
        return File(dir, "p2p.sock")
    }

    private fun registerConnectivityCallback() {
        if (connectivityCallback != null) return
        val cm =
            applicationContext.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                ?: return
        val callback =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    lastDefaultNetwork = network
                    notifyP2pNetworkChange()
                }

                override fun onLost(network: Network) {
                    if (lastDefaultNetwork == network) {
                        lastDefaultNetwork = null
                        notifyP2pNetworkChange()
                    }
                }

                override fun onCapabilitiesChanged(
                    network: Network,
                    networkCapabilities: NetworkCapabilities,
                ) {
                    // Some devices keep the same Network object while switching transports.
                    val key =
                        buildString {
                            append(if (networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) "wifi" else "")
                            append("|")
                            append(if (networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) "cell" else "")
                            append("|")
                            append(if (networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) "eth" else "")
                        }
                    val transportChanged = key != lastTransportsKey
                    if (transportChanged) {
                        lastTransportsKey = key
                        lastDefaultNetwork = network
                        notifyP2pNetworkChange()
                    }
                }
            }
        connectivityCallback = callback
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                cm.registerDefaultNetworkCallback(callback, mainHandler)
            } else {
                cm.registerNetworkCallback(NetworkRequest.Builder().build(), callback, mainHandler)
            }
            registerWifiNetworkCallback(cm)
            notifyP2pNetworkChange()
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "connectivity callback: ${e.message}")
            connectivityCallback = null
        }
    }

    private fun unregisterConnectivityCallback() {
        val cm =
            applicationContext.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                ?: return
        connectivityCallback?.let {
            try {
                cm.unregisterNetworkCallback(it)
            } catch (_: Throwable) {
            }
        }
        connectivityCallback = null
        wifiConnectivityCallback?.let {
            try {
                cm.unregisterNetworkCallback(it)
            } catch (_: Throwable) {
            }
        }
        wifiConnectivityCallback = null
    }

    /** Wi‑Fi link up/down even when cellular remains the default network. */
    private fun registerWifiNetworkCallback(cm: ConnectivityManager) {
        if (wifiConnectivityCallback != null) return
        val request =
            NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .build()
        val callback =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = notifyP2pNetworkChange()

                override fun onLost(network: Network) = notifyP2pNetworkChange()
            }
        wifiConnectivityCallback = callback
        try {
            cm.registerNetworkCallback(request, callback, mainHandler)
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "wifi network callback: ${e.message}")
            wifiConnectivityCallback = null
        }
    }

    private fun notifyP2pNetworkChange() {
        mainHandler.removeCallbacks(networkNotifyRunnable)
        // Coalesce bursts (Android emits many callbacks during handover) without adding seconds of delay.
        mainHandler.postDelayed(networkNotifyRunnable, 500)
    }

    private fun startDaemonThreadIfNeeded() {
        if (daemonThread?.isAlive == true) return
        // Full libp2p flow on the terminal (debug builds only): the `:p2p` process reads
        // GHAL_BOL_VERBOSE_LOG once on the first native_log call, so set it before runDaemon.
        // Forwards Rust `debug` lines through the log sink → App log → `flutter run` terminal.
        // Use the debuggable flag (BuildConfig is disabled by default under AGP 8).
        val debuggable =
            (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
        if (debuggable) {
            try {
                android.system.Os.setenv("GHAL_BOL_VERBOSE_LOG", "1", true)
            } catch (e: Throwable) {
                android.util.Log.w("GhalBol", "setenv GHAL_BOL_VERBOSE_LOG: ${e.message}")
            }
        }
        val sock = socketFile()
        if (sock.exists()) {
            try {
                sock.delete()
            } catch (_: Throwable) {
            }
        }
        val path = sock.absolutePath
        daemonThread =
            Thread(
                {
                    try {
                        P2pDaemonNative.runDaemon(path)
                    } catch (e: Throwable) {
                        android.util.Log.e("GhalBol", "daemon thread: ${e.message}")
                    }
                },
                "ghal_bol_p2p_daemon",
            ).apply {
                isDaemon = true
                start()
            }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP_FOR_LOGOUT) {
            userRequestedStop = true
            mainHandler.removeCallbacks(restartRunnable)
            mainHandler.removeCallbacks(networkNotifyRunnable)
            cancelUnlockNotification()
            stopSelf()
            return START_NOT_STICKY
        }
        userRequestedStop = false
        promoteForegroundNotification()
        configureNativeDataDir()
        acquireWakeLock()
        acquireWifiLock()
        acquireMulticastLock()
        startDaemonThreadIfNeeded()
        registerConnectivityCallback()
        val bootOrRestart = intent == null || intent.getBooleanExtra(EXTRA_BOOT_START, false)
        if (bootOrRestart) postUnlockNotificationIfNeeded()
        return START_STICKY
    }

    private fun postUnlockNotificationIfNeeded() {
        if (!BootReceiver.hasKeystore(applicationContext)) return
        val nm = getSystemService(NOTIFICATION_SERVICE) as? NotificationManager ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(
                UNLOCK_CHANNEL_ID,
                "Unlock prompt",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Prompts you to unlock Ghal Bol after device restart"
            }
            nm.createNotificationChannel(ch)
        }
        val open = PendingIntent.getActivity(
            this,
            1,
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = NotificationCompat.Builder(this, UNLOCK_CHANNEL_ID)
            .setContentTitle("Ghal Bol")
            .setContentText("Enter your password to start receiving messages")
            .setSmallIcon(R.drawable.ic_notification)
            .setContentIntent(open)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_REMINDER)
            .build()
        nm.notify(UNLOCK_NOTIFICATION_ID, notification)
    }

    fun cancelUnlockNotification() {
        try {
            (getSystemService(NOTIFICATION_SERVICE) as? NotificationManager)
                ?.cancel(UNLOCK_NOTIFICATION_ID)
        } catch (_: Throwable) {}
    }

    private fun promoteForegroundNotification() {
        val channelId = "ghal_bol_p2p_v1"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch =
                NotificationChannel(
                    channelId,
                    "Ghal Bol — P2P listener",
                    NotificationManager.IMPORTANCE_LOW,
                ).apply {
                    description = "Keeps encrypted chat networking active in the background."
                    setShowBadge(false)
                }
            (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).createNotificationChannel(ch)
        }

        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )

        val notification =
            NotificationCompat.Builder(this, channelId)
                .setContentTitle("Ghal Bol")
                .setContentText("Listening for messages")
                .setSmallIcon(R.drawable.ic_notification)
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setCategory(NotificationCompat.CATEGORY_SERVICE)
                .build()

        val baseType =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING
            } else {
                0
            }
        // Native voice (P6) records the mic from `:p2p`; this needs the microphone FGS
        // type, which is only legal once RECORD_AUDIO is granted. If we promote with it
        // from the background, Android 14+ may reject it — fall back to base type then.
        val micGranted =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                ContextCompat.checkSelfPermission(
                    this,
                    android.Manifest.permission.RECORD_AUDIO,
                ) == android.content.pm.PackageManager.PERMISSION_GRANTED
        val cameraGranted =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                ContextCompat.checkSelfPermission(
                    this,
                    android.Manifest.permission.CAMERA,
                ) == android.content.pm.PackageManager.PERMISSION_GRANTED
        var fgType =
            if (micGranted) {
                baseType or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            } else {
                baseType
            }
        if (cameraGranted && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            fgType = fgType or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        }
        try {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, fgType)
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "startForeground(mic=$micGranted): ${e.message}")
            try {
                ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, baseType)
            } catch (e2: Throwable) {
                android.util.Log.e("GhalBol", "startForeground fallback: ${e2.message}")
            }
        }
    }

    private fun scheduleRestart() {
        if (userRequestedStop) return
        mainHandler.removeCallbacks(restartRunnable)
        mainHandler.postDelayed(restartRunnable, 800)
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        try {
            ContextCompat.startForegroundService(
                applicationContext,
                Intent(applicationContext, GhalBolP2pService::class.java),
            )
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "restart p2p service after task removed: ${e.message}")
        }
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        mainHandler.removeCallbacks(networkNotifyRunnable)
        unregisterConnectivityCallback()
        releaseMulticastLock()
        releaseWifiLock()
        releaseWakeLock()
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        val restart = !userRequestedStop
        if (!restart) {
            mainHandler.removeCallbacks(restartRunnable)
        }
        super.onDestroy()
        if (restart) {
            scheduleRestart()
        } else {
            userRequestedStop = false
        }
    }

    companion object {
        const val ACTION_STOP_FOR_LOGOUT = "com.ghalbol.STOP_P2P_LOGOUT"
        const val EXTRA_BOOT_START = "com.ghalbol.BOOT_START"

        @Volatile
        private var userRequestedStop = false

        private const val NOTIFICATION_ID = 0x6768_6c62
        private const val UNLOCK_CHANNEL_ID = "ghalbol_unlock"
        private const val UNLOCK_NOTIFICATION_ID = 0x6768_756e

        fun stopIntent(context: Context): Intent =
            Intent(context, GhalBolP2pService::class.java).setAction(ACTION_STOP_FOR_LOGOUT)

        fun socketPath(context: Context): String {
            val dir = File(context.filesDir, "ghalbol")
            return File(dir, "p2p.sock").absolutePath
        }

        fun cancelUnlockNotification(context: Context) {
            try {
                (context.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager)
                    ?.cancel(UNLOCK_NOTIFICATION_ID)
            } catch (_: Throwable) {}
        }
    }
}
