package com.ghalbol

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.view.WindowManager
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {

    companion object {
        const val ACTION_INCOMING_CALL = "com.ghalbol.INCOMING_CALL"
        const val EXTRA_CALLER_NAME = "caller_name"
        const val EXTRA_CALLER_PK = "caller_pk"
    }

    private var incomingCallChannel: MethodChannel? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ghal_bol/embedder")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "dataRootForFfi" ->
                        result.success(NativeStorage.dataRoot(applicationContext).absolutePath)
                    "isBatteryOptimized" -> {
                        result.success(isBatteryOptimized())
                    }
                    "requestBatteryOptimizationExemption" -> {
                        requestBatteryOptimizationExemption()
                        result.success(null)
                    }
                    "isUnusedAppPauseEnabled" -> {
                        result.success(isUnusedAppPauseEnabled())
                    }
                    "openUnusedAppSettings" -> {
                        openUnusedAppSettings()
                        result.success(null)
                    }
                    "cancelUnlockNotification" -> {
                        GhalBolP2pService.cancelUnlockNotification(applicationContext)
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ghal_bol/p2p_daemon")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "startP2pService" -> {
                        try {
                            ContextCompat.startForegroundService(
                                applicationContext,
                                Intent(applicationContext, GhalBolP2pService::class.java),
                            )
                            result.success(GhalBolP2pService.socketPath(applicationContext))
                        } catch (e: Throwable) {
                            result.error("start_failed", e.message, null)
                        }
                    }
                    "getSocketPath" -> {
                        result.success(GhalBolP2pService.socketPath(applicationContext))
                    }
                    "stopP2pService" -> {
                        try {
                            startService(GhalBolP2pService.stopIntent(applicationContext))
                            result.success(null)
                        } catch (e: Throwable) {
                            result.error("stop_failed", e.message, null)
                        }
                    }
                    else -> result.notImplemented()
                }
            }

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ghal_bol/listener")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "startForeground" -> {
                        try {
                            ContextCompat.startForegroundService(
                                applicationContext,
                                Intent(applicationContext, GhalBolP2pService::class.java),
                            )
                            result.success(null)
                        } catch (e: Throwable) {
                            result.error("start_failed", e.message, null)
                        }
                    }
                    "stopForeground" -> {
                        try {
                            startService(GhalBolP2pService.stopIntent(applicationContext))
                            result.success(null)
                        } catch (e: Throwable) {
                            result.error("stop_failed", e.message, null)
                        }
                    }
                    else -> result.notImplemented()
                }
            }

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ghal_bol/call_video_texture")
            .setMethodCallHandler { call, result ->
                val registry = flutterEngine.renderer
                when (call.method) {
                    "register" -> {
                        val shmPath = call.argument<String>("shmPath")
                        val width = call.argument<Int>("width") ?: 0
                        val height = call.argument<Int>("height") ?: 0
                        if (shmPath.isNullOrBlank()) {
                            result.error("bad_args", "shmPath required", null)
                        } else {
                            val id =
                                CallVideoTexture.register(
                                    registry,
                                    shmPath,
                                    width,
                                    height,
                                )
                            result.success(id)
                        }
                    }
                    "release" -> {
                        val textureId = call.argument<Number>("textureId")?.toLong()
                        if (textureId == null) {
                            result.error("bad_args", "textureId required", null)
                        } else {
                            CallVideoTexture.release(textureId)
                            result.success(null)
                        }
                    }
                    "releaseAll" -> {
                        CallVideoTexture.releaseAll()
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }

        incomingCallChannel =
            MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ghal_bol/incoming_call")
                .also { channel ->
                    channel.setMethodCallHandler { call, result ->
                        when (call.method) {
                            "show" -> {
                                val name = call.argument<String>("displayName") ?: "Contact"
                                val pk = call.argument<String>("publicKeyHex") ?: ""
                                IncomingCallNotifier.show(applicationContext, name, pk)
                                result.success(null)
                            }
                            "dismiss" -> {
                                IncomingCallNotifier.dismiss(applicationContext)
                                result.success(null)
                            }
                            else -> result.notImplemented()
                        }
                    }
                }

        deliverIncomingCallIntent(intent)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prepareForIncomingCallIntent(intent)
        deliverIncomingCallIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        prepareForIncomingCallIntent(intent)
        deliverIncomingCallIntent(intent)
    }

    private fun prepareForIncomingCallIntent(intent: Intent?) {
        if (intent?.action != ACTION_INCOMING_CALL) return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(
                WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or
                    WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON,
            )
        }
    }

    private fun deliverIncomingCallIntent(intent: Intent?) {
        if (intent?.action != ACTION_INCOMING_CALL) return
        val pk = intent.getStringExtra(EXTRA_CALLER_PK) ?: ""
        val name = intent.getStringExtra(EXTRA_CALLER_NAME) ?: "Contact"
        incomingCallChannel?.invokeMethod(
            "openedFromNotification",
            mapOf("publicKeyHex" to pk, "displayName" to name),
        )
    }

    private fun isBatteryOptimized(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return false
        return try {
            val pm = getSystemService(POWER_SERVICE) as? PowerManager ?: return false
            !pm.isIgnoringBatteryOptimizations(packageName)
        } catch (_: Throwable) {
            false
        }
    }

    @Suppress("BatteryLife")
    private fun requestBatteryOptimizationExemption() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return
        try {
            val pm = getSystemService(POWER_SERVICE) as? PowerManager ?: return
            if (pm.isIgnoringBatteryOptimizations(packageName)) return
            startActivity(
                Intent(
                    Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                    Uri.parse("package:$packageName"),
                ),
            )
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "requestBatteryOptExemption: ${e.message}")
        }
    }

    private fun isUnusedAppPauseEnabled(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return false
        return try {
            !packageManager.isAutoRevokeWhitelisted
        } catch (_: Throwable) {
            false
        }
    }

    private fun openUnusedAppSettings() {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                        data = Uri.fromParts("package", packageName, null)
                    },
                )
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                startActivity(
                    Intent(Intent.ACTION_AUTO_REVOKE_PERMISSIONS).apply {
                        data = Uri.fromParts("package", packageName, null)
                    },
                )
            }
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "openUnusedAppSettings: ${e.message}")
            try {
                startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                        data = Uri.fromParts("package", packageName, null)
                    },
                )
            } catch (_: Throwable) {}
        }
    }
}
