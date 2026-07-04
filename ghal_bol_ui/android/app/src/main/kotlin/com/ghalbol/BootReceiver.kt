package com.ghalbol

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import java.io.File

/**
 * Starts [GhalBolP2pService] on device boot when the user already has a keystore
 * (identity created / imported previously). The daemon starts locked — it posts an
 * unlock notification so the user can open the app and enter their password.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        if (intent?.action != Intent.ACTION_BOOT_COMPLETED) return
        if (!hasKeystore(context)) return
        try {
            ContextCompat.startForegroundService(
                context,
                Intent(context, GhalBolP2pService::class.java)
                    .putExtra(GhalBolP2pService.EXTRA_BOOT_START, true),
            )
        } catch (e: Throwable) {
            android.util.Log.w("GhalBol", "boot start p2p service: ${e.message}")
        }
    }

    companion object {
        fun hasKeystore(context: Context): Boolean {
            val dataRoot = NativeStorage.dataRoot(context)
            if (File(dataRoot, "keystore_v1.json").exists()) return true
            if (File(dataRoot, "${context.packageName}/keystore_v1.json").exists()) return true
            return false
        }
    }
}
