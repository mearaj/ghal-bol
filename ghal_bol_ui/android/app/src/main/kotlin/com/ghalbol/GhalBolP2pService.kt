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
import android.os.Looper
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import java.io.File

/**
 * Foreground service in process `:p2p` — hosts libp2p + JSON-RPC over a Unix socket.
 */
class GhalBolP2pService : Service() {

    private var multicastLock: WifiManager.MulticastLock? = null
    private var daemonThread: Thread? = null
    private var connectivityCallback: ConnectivityManager.NetworkCallback? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private var lastDefaultNetwork: Network? = null
    private var lastTransportsKey: String = ""
    private val networkNotifyRunnable = Runnable {
        try {
            P2pDaemonNative.notifyNetworkChange()
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "notifyNetworkChange: ${e.message}")
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun configureNativeDataDir() {
        val dir = NativeStorage.dataRoot(this).absolutePath
        try {
            P2pDaemonNative.initTls(applicationContext)
            P2pDaemonNative.configureDataDirectory(dir)
            android.util.Log.i("GhalBol", "p2p data dir=$dir")
        } catch (e: Throwable) {
            android.util.Log.e("GhalBol", "configureDataDirectory: ${e.message}")
        }
    }

    private fun acquireMulticastLock() {
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
    }

    private fun notifyP2pNetworkChange() {
        mainHandler.removeCallbacks(networkNotifyRunnable)
        // Coalesce bursts (Android emits many callbacks during handover) without adding seconds of delay.
        mainHandler.postDelayed(networkNotifyRunnable, 500)
    }

    private fun startDaemonThreadIfNeeded() {
        if (daemonThread?.isAlive == true) return
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
        configureNativeDataDir()
        acquireMulticastLock()
        startDaemonThreadIfNeeded()
        registerConnectivityCallback()

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

        val fgType =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING
            } else {
                0
            }
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, fgType)

        return START_STICKY
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
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        private const val NOTIFICATION_ID = 0x6768_6c62

        fun socketPath(context: Context): String {
            val dir = File(context.filesDir, "ghalbol")
            return File(dir, "p2p.sock").absolutePath
        }
    }
}
