package com.ghalbol

import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.util.Log

/**
 * Checks and intents for keeping `:p2p` alive with the screen off on stock Android and common OEMs.
 * Query when the OS exposes state; otherwise offer a one-time settings shortcut per manufacturer.
 */
object BackgroundReadiness {
    private const val TAG = "GhalBol"
    private const val PREFS = "ghal_bol_bg_readiness"

    fun isBatteryOptimized(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return false
        return try {
            val pm = context.getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return false
            !pm.isIgnoringBatteryOptimizations(context.packageName)
        } catch (_: Throwable) {
            false
        }
    }

    @Suppress("BatteryLife")
    fun requestBatteryOptimizationExemption(activity: Activity) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return
        try {
            val pm = activity.getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return
            if (pm.isIgnoringBatteryOptimizations(activity.packageName)) return
            activity.startActivity(
                Intent(
                    Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                    Uri.parse("package:${activity.packageName}"),
                ),
            )
        } catch (e: Throwable) {
            Log.w(TAG, "requestBatteryOptExemption: ${e.message}")
            openBatteryOptimizationList(activity)
        }
    }

    private fun openBatteryOptimizationList(activity: Activity) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return
        try {
            activity.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
        } catch (e: Throwable) {
            Log.w(TAG, "openBatteryOptimizationList: ${e.message}")
        }
    }

    fun isUnusedAppPauseEnabled(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return false
        return try {
            !context.packageManager.isAutoRevokeWhitelisted
        } catch (_: Throwable) {
            false
        }
    }

    fun openUnusedAppSettings(activity: Activity) {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                activity.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                        data = Uri.fromParts("package", activity.packageName, null)
                    },
                )
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                activity.startActivity(
                    Intent(Intent.ACTION_AUTO_REVOKE_PERMISSIONS).apply {
                        data = Uri.fromParts("package", activity.packageName, null)
                    },
                )
            }
        } catch (e: Throwable) {
            Log.w(TAG, "openUnusedAppSettings: ${e.message}")
            openAppDetails(activity)
        }
    }

    fun openAppDetails(activity: Activity) {
        try {
            activity.startActivity(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                    data = Uri.fromParts("package", activity.packageName, null)
                },
            )
        } catch (_: Throwable) {}
    }

    /** Step ids still required on the native side (`battery`, `unused_pause`, `oem_background`). */
    fun pendingNativeStepIds(context: Context): List<String> {
        val out = ArrayList<String>(3)
        if (isBatteryOptimized(context)) out.add("battery")
        if (isUnusedAppPauseEnabled(context)) out.add("unused_pause")
        if (needsOemBackgroundStep(context)) out.add("oem_background")
        return out
    }

    fun needsOemBackgroundStep(context: Context): Boolean {
        if (isOemBackgroundSatisfied(context)) return false
        if (oemPrefs(context).getBoolean(oemAckKey(), false)) return false
        return resolveOemBackgroundIntent(context) != null
    }

    fun isOemBackgroundSatisfied(context: Context): Boolean =
        when (manufacturerKey()) {
            "vivo" -> isVivoAutostartAllowed(context)
            else -> false
        }

    fun openOemBackgroundSettings(activity: Activity): Boolean {
        val intent = resolveOemBackgroundIntent(activity) ?: return false
        return try {
            activity.startActivity(intent)
            true
        } catch (e: Throwable) {
            Log.w(TAG, "openOemBackgroundSettings: ${e.message}")
            false
        }
    }

    fun markOemBackgroundStepAcknowledged(context: Context) {
        oemPrefs(context).edit().putBoolean(oemAckKey(), true).apply()
    }

    private fun resolveOemBackgroundIntent(context: Context): Intent? {
        val pkg = context.packageName
        val mfg = Build.MANUFACTURER.lowercase()
        val candidates =
            when {
                mfg.contains("xiaomi") || mfg.contains("redmi") || mfg.contains("poco") ->
                    listOf(
                        componentIntent(
                            "com.miui.securitycenter",
                            "com.miui.permcenter.autostart.AutoStartManagementActivity",
                        ),
                    )
                mfg.contains("oppo") || mfg.contains("realme") ->
                    listOf(
                        componentIntent(
                            "com.coloros.safecenter",
                            "com.coloros.safecenter.permission.startup.StartupAppListActivity",
                        ),
                        componentIntent(
                            "com.oppo.safe",
                            "com.oppo.safe.permission.startup.StartupAppListActivity",
                        ),
                    )
                mfg.contains("vivo") ->
                    listOf(
                        componentIntent(
                            "com.vivo.permissionmanager",
                            "com.vivo.permissionmanager.activity.BgStartUpManagerActivity",
                        ),
                        componentIntent(
                            "com.iqoo.secure",
                            "com.iqoo.secure.ui.phoneoptimize.AddWhiteListActivity",
                        ),
                    )
                mfg.contains("huawei") || mfg.contains("honor") ->
                    listOf(
                        componentIntent(
                            "com.huawei.systemmanager",
                            "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity",
                        ),
                    )
                mfg.contains("oneplus") ->
                    listOf(
                        componentIntent(
                            "com.oneplus.security",
                            "com.oneplus.security.chainlaunch.view.ChainLaunchAppListActivity",
                        ),
                    )
                mfg.contains("asus") ->
                    listOf(
                        componentIntent(
                            "com.asus.mobilemanager",
                            "com.asus.mobilemanager.autostart.AutoStartActivity",
                        ),
                    )
                else -> emptyList()
            }
        for (intent in candidates) {
            if (canResolve(context, intent)) {
                intent.putExtra("packageName", pkg)
                intent.putExtra("package_name", pkg)
                return intent
            }
        }
        return null
    }

    private fun componentIntent(pkg: String, cls: String): Intent =
        Intent().setComponent(ComponentName(pkg, cls))

    private fun canResolve(context: Context, intent: Intent): Boolean =
        context.packageManager.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY) != null

    private fun isVivoAutostartAllowed(context: Context): Boolean {
        val pkg = context.packageName
        val uris =
            listOf(
                "content://com.vivo.permissionmanager.provider.permission/bg_start_up_apps",
                "content://com.iqoo.secure.provider.secureprovider/allowbgstartapp",
            )
        for (uriStr in uris) {
            try {
                val uri = Uri.parse(uriStr)
                context.contentResolver.query(
                    uri,
                    null,
                    "pkgname = ?",
                    arrayOf(pkg),
                    null,
                )?.use { cursor ->
                    if (!cursor.moveToFirst()) return@use
                    val idx = cursor.getColumnIndex("currentstate")
                    if (idx >= 0) {
                        return cursor.getInt(idx) == 0
                    }
                    val allowedIdx = cursor.getColumnIndex("allowed")
                    if (allowedIdx >= 0) {
                        return cursor.getInt(allowedIdx) == 1
                    }
                }
            } catch (_: Throwable) {
                // try next URI
            }
        }
        return false
    }

    private fun manufacturerKey(): String = Build.MANUFACTURER.lowercase().trim()

    private fun oemAckKey(): String = "oem_ack_${manufacturerKey()}"

    private fun oemPrefs(context: Context) =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
