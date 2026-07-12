package com.ghalbol

import android.content.Context
import java.io.File

/** Single Android app-private root for `lib_ghal_bol_core` (UI and `:p2p` process). */
object NativeStorage {
    fun dataRoot(context: Context): File = File(context.applicationInfo.dataDir, "app_flutter")
}
