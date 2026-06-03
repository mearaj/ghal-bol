package com.ghalbol

import android.content.Intent
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ghal_bol/embedder")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "dataRootForFfi" ->
                        result.success(NativeStorage.dataRoot(applicationContext).absolutePath)
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
    }
}
