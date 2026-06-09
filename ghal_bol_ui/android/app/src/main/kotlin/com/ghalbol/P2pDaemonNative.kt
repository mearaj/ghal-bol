package com.ghalbol

import android.content.Context

/**
 * Loads `libghal_bol.so` in the `:p2p` process and runs the Unix-socket P2P daemon (Rust).
 */
object P2pDaemonNative {
    init {
        System.loadLibrary("ghal_bol")
    }

    /** Call before coord HTTPS / `reqwest` in the `:p2p` process (once per process). */
    @JvmStatic
    external fun initRustlsPlatformVerifier(context: Context)

    @JvmStatic
    fun initTls(context: Context) {
        initRustlsPlatformVerifier(context.applicationContext)
    }

    /** Hand cpal/Oboe the JavaVM + Context so native voice can open mic/speaker in `:p2p`. */
    @JvmStatic
    external fun initAndroidAudio(context: Context)

    /** Camera2 I420 frames from `:p2p` → native video engine (called from Kotlin). */
    @JvmStatic
    external fun pushCameraFrame(data: ByteArray, width: Int, height: Int)

    @JvmStatic
    fun startCameraCapture(context: Context) {
        AndroidVideoCapture.start(context.applicationContext)
    }

    @JvmStatic
    fun stopCameraCapture() {
        AndroidVideoCapture.stop()
    }

    @JvmStatic
    fun initAudio(context: Context) {
        initAndroidAudio(context.applicationContext)
    }

    @JvmStatic
    external fun configureDataDirectory(absolutePath: String)

    /** Blocks until the daemon exits; call from a background thread only. */
    @JvmStatic
    external fun runDaemon(socketAbsolutePath: String): Boolean

    /** Hint libp2p that Wi‑Fi/mobile/default route changed (call from `:p2p` process). */
    @JvmStatic
    external fun notifyNetworkChange()
}
