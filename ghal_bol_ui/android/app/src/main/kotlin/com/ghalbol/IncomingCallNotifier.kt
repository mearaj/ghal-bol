package com.ghalbol

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat

/** Full-screen incoming call notification — wakes UI when the app is backgrounded. */
object IncomingCallNotifier {
    private const val CHANNEL_ID = "ghal_bol_incoming_call_v1"
    private const val NOTIFICATION_ID = 9001

    fun show(context: Context, callerName: String, callerPk: String) {
        ensureChannel(context)
        val launch =
            Intent(context, MainActivity::class.java).apply {
                action = MainActivity.ACTION_INCOMING_CALL
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP
                putExtra(MainActivity.EXTRA_CALLER_NAME, callerName)
                putExtra(MainActivity.EXTRA_CALLER_PK, callerPk)
            }
        val pending =
            PendingIntent.getActivity(
                context,
                0,
                launch,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            NotificationCompat.Builder(context, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("Incoming call")
                .setContentText(callerName)
                .setPriority(NotificationCompat.PRIORITY_MAX)
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
                .setOngoing(true)
                .setAutoCancel(false)
                .setFullScreenIntent(pending, true)
                .setContentIntent(pending)
                .build()
        NotificationManagerCompat.from(context).notify(NOTIFICATION_ID, notification)
    }

    fun dismiss(context: Context) {
        NotificationManagerCompat.from(context).cancel(NOTIFICATION_ID)
    }

    private fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val ch =
            NotificationChannel(
                CHANNEL_ID,
                "Incoming calls",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Alerts when someone calls you on Ghal Bol"
                setBypassDnd(true)
                lockscreenVisibility = NotificationCompat.VISIBILITY_PUBLIC
            }
        (context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager)
            .createNotificationChannel(ch)
    }
}
