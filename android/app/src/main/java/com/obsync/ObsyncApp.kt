package com.obsync

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager

class ObsyncApp : Application() {
    override fun onCreate() {
        super.onCreate()
        instance = this
        createNotificationChannel()
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            SYNC_CHANNEL_ID,
            "Sync Status",
            NotificationManager.IMPORTANCE_LOW
        ).apply { description = "Shows current sync state" }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
    }

    companion object {
        const val SYNC_CHANNEL_ID = "obsync-sync"
        lateinit var instance: ObsyncApp
            private set
    }
}
