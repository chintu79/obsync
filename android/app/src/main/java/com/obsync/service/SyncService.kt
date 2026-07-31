package com.obsync.service

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.obsync.ObsyncApp
import com.obsync.bridge.RustBridge
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject

enum class SyncServiceState { Idle, Connecting, Syncing, Offline }

class SyncService : Service() {
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private val _state = MutableStateFlow(SyncServiceState.Idle)

    companion object {
        const val TAG = "Obsync/SyncService"
        const val NOTIFICATION_ID = 1001
        const val EXTRA_ADDR = "addr"
        const val EXTRA_PORT = "port"
        const val EXTRA_VAULT = "vault"
    }

    override fun onCreate() {
        super.onCreate()
        startForeground(NOTIFICATION_ID, notification("Starting..."))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val addr = intent?.getStringExtra(EXTRA_ADDR) ?: ""
        val port = intent?.getIntExtra(EXTRA_PORT, 42042) ?: 42042
        val vault = intent?.getStringExtra(EXTRA_VAULT) ?: ""
        if (addr.isBlank() || vault.isBlank()) {
            updateNotification("No peer configured")
            return START_NOT_STICKY
        }
        scope.launch { runSyncCycle(addr, port, vault) }
        return START_NOT_STICKY
    }

    private suspend fun runSyncCycle(addr: String, port: Int, vault: String) {
        while (currentCoroutineContext().isActive) {
            try {
                _state.value = SyncServiceState.Connecting
                updateNotification("Connecting to $addr...")

                _state.value = SyncServiceState.Syncing
                updateNotification("Syncing...")

                val identity = buildIdentityJson()
                val result = withContext(Dispatchers.Default) {
                    RustBridge.syncOnce(addr, port, vault, identity)
                }
                val json = JSONObject(result)
                if (json.has("error")) {
                    Log.w(TAG, "Sync error: ${json.getString("error")}")
                    updateNotification("Sync failed: ${json.getString("error")}")
                } else {
                    val pulled = json.optInt("pulled")
                    val pushed = json.optInt("pushed")
                    val deleted = json.optInt("deleted")
                    val conflicts = json.optInt("conflicts")
                    Log.i(TAG, "Synced: +$pulled -$deleted ^$pushed conflicts=$conflicts")
                    updateNotification("Synced: +$pulled, -$deleted, $conflicts conflicts")
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(TAG, "Sync error", e)
                _state.value = SyncServiceState.Offline
                updateNotification("Offline — ${e.message ?: e.javaClass.simpleName}")
            }
            _state.value = SyncServiceState.Idle
            delay(30_000)
        }
    }

    private fun buildIdentityJson(): String {
        return try {
            val storePath = "${filesDir.absolutePath}/identity.bin"
            val data = RustBridge.loadOrCreateIdentity("Android", storePath)
            val json = JSONObject(String(data))
            JSONObject().apply {
                put("device_id", json.getString("device_id"))
                put("device_name", json.getString("device_name"))
                put("fingerprint", json.getString("fingerprint"))
            }.toString()
        } catch (_: Exception) {
            """{"device_id":"android","device_name":"Android","fingerprint":""}"""
        }
    }

    private fun updateNotification(text: String) {
        val notification = notification(text)
        val manager = getSystemService(NOTIFICATION_SERVICE) as android.app.NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
    }

    private fun notification(text: String) = NotificationCompat.Builder(this, ObsyncApp.SYNC_CHANNEL_ID)
        .setContentTitle("Obsync")
        .setContentText(text)
        .setSmallIcon(com.obsync.R.mipmap.ic_launcher)
        .setOngoing(true)
        .setPriority(NotificationCompat.PRIORITY_LOW)
        .build()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        scope.cancel()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }
}
