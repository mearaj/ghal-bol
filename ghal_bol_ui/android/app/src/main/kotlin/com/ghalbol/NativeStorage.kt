package com.ghalbol

import android.content.Context
import java.io.File

/** Single Android app-private root for `libghal_bol` (UI and `:p2p` process). */
object NativeStorage {
    fun dataRoot(context: Context): File = File(context.applicationInfo.dataDir, "app_flutter")
}
