package com.obsync.service

import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.FileObserver
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.obsync.ObsyncApp
import com.obsync.bridge.RustBridge
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject
import java.io.File

enum class SyncServiceStatus { Idle, Connecting, Syncing, Offline }

data class SyncServiceState(
    val status: SyncServiceStatus = SyncServiceStatus.Idle,
    val lastSync: String = "",
)

class SyncService : Service() {
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private val fileObservers = HashMap<String, FileObserver>()

    companion object {
        const val TAG = "Obsync/SyncService"
        const val NOTIFICATION_ID = 1001
        const val EXTRA_ADDR = "addr"
        const val EXTRA_PORT = "port"
        const val EXTRA_VAULT = "vault"
        const val SYNC_INTERVAL_MS = 30_000L
        const val POST_SYNC_EVENT_SUPPRESS_MS = 3_000L

        val state = MutableStateFlow(SyncServiceState())

        private val syncTrigger = Channel<Unit>(Channel.CONFLATED)

        @Volatile private var loopActive = false
        @Volatile private var currentAddr = ""
        @Volatile private var currentPort = 42042
        @Volatile private var currentVault = ""
        @Volatile private var suppressEventsUntil = 0L

        /** Request an immediate sync (conflated — at most one pending). */
        fun syncNow() {
            syncTrigger.trySend(Unit)
        }

        fun isRunning() = loopActive
    }

    override fun onCreate() {
        super.onCreate()
        startForeground(NOTIFICATION_ID, notification("Starting..."))
        registerNetworkCallback()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // On a system restart the intent is null — fall back to the last config.
        val addr = intent?.getStringExtra(EXTRA_ADDR) ?: currentAddr
        val port = intent?.getIntExtra(EXTRA_PORT, 42042) ?: currentPort
        val vault = intent?.getStringExtra(EXTRA_VAULT) ?: currentVault
        if (addr.isBlank() || vault.isBlank()) {
            updateNotification("No peer configured")
            return START_NOT_STICKY
        }
        currentAddr = addr
        currentPort = port
        currentVault = vault
        if (!loopActive) {
            loopActive = true
            scope.launch { runSyncCycle() }
            scope.launch { watchVault(vault) }
        }
        syncNow() // sync right away on start
        return START_STICKY
    }

    private suspend fun runSyncCycle() {
        while (currentCoroutineContext().isActive) {
            val addr = currentAddr
            val port = currentPort
            val vault = currentVault
            try {
                state.value = SyncServiceState(SyncServiceStatus.Connecting)
                updateNotification("Connecting to $addr...")

                state.value = SyncServiceState(SyncServiceStatus.Syncing)
                updateNotification("Syncing...")

                val identity = buildIdentityJson()
                val result = withContext(Dispatchers.Default) {
                    RustBridge.syncOnce(addr, port, vault, identity)
                }
                val json = JSONObject(result)
                if (json.has("error")) {
                    Log.w(TAG, "Sync error: ${json.getString("error")}")
                    state.value = SyncServiceState(SyncServiceStatus.Offline, "Failed: ${json.getString("error")}")
                    updateNotification("Sync failed: ${json.getString("error")}")
                } else {
                    val pulled = json.optInt("pulled")
                    val pushed = json.optInt("pushed")
                    val deleted = json.optInt("deleted")
                    val conflicts = json.optInt("conflicts")
                    Log.i(TAG, "Synced: +$pulled -$deleted ^$pushed conflicts=$conflicts")
                    val summary = buildString {
                        append("Synced ")
                        append(
                            when {
                                pulled == 1 -> "1 file"
                                pulled != 0 -> "$pulled files"
                                else -> "no files"
                            }
                        )
                        if (deleted > 0) append(", $deleted removed")
                        if (conflicts > 0) append(", $conflicts conflict" + if (conflicts == 1) "" else "s")
                    }
                    state.value = SyncServiceState(SyncServiceStatus.Idle, summary)
                    updateNotification(summary)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(TAG, "Sync error", e)
                state.value = SyncServiceState(SyncServiceStatus.Offline, "Offline — ${e.message ?: e.javaClass.simpleName}")
                updateNotification("Offline — ${e.message ?: e.javaClass.simpleName}")
            }
            // Our own writes fire file events right after a sync — suppress them
            // briefly, then wait for the next trigger (event, network change,
            // foreground) or the periodic interval.
            suppressEventsUntil = System.currentTimeMillis() + POST_SYNC_EVENT_SUPPRESS_MS
            withTimeoutOrNull(SYNC_INTERVAL_MS) { syncTrigger.receive() }
        }
    }

    /** Watch the vault (recursively) for file changes and trigger an immediate sync. */
    private suspend fun watchVault(vaultPath: String) {
        val root = File(vaultPath)
        if (!root.isDirectory) return
        val dirs = ArrayDeque<File>()
        dirs.add(root)
        while (dirs.isNotEmpty()) {
            val dir = dirs.removeFirst()
            startObserving(dir)
            runCatching { dir.listFiles()?.filter { it.isDirectory }?.let(dirs::addAll) }
        }
    }

    private fun startObserving(dir: File) {
        val path = dir.absolutePath
        if (fileObservers.containsKey(path)) return
        val observer = object : FileObserver(path, MODIFY or CREATE or DELETE or MOVED_FROM or MOVED_TO) {
            override fun onEvent(event: Int, eventPath: String?) {
                if (event and FileObserver.ALL_EVENTS == 0) return
                if (System.currentTimeMillis() < suppressEventsUntil) return
                val affected = File(dir, eventPath ?: "")
                if (affected.isDirectory) startObserving(affected)
                syncNow()
            }
        }
        try {
            observer.startWatching()
            fileObservers[path] = observer
        } catch (_: Exception) {
            Log.w(TAG, "FileObserver failed for $path")
        }
    }

    private fun registerNetworkCallback() {
        try {
            val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
            val callback = object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = syncNow()
                override fun onLost(network: Network) {
                    state.value = SyncServiceState(SyncServiceStatus.Offline, "Network lost")
                }
                override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                    if (caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) {
                        syncNow()
                    }
                }
            }
            cm.registerDefaultNetworkCallback(callback)
            networkCallback = callback
        } catch (e: Exception) {
            Log.w(TAG, "Network callback failed", e)
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
        state.value = SyncServiceState()
        loopActive = false
        fileObservers.values.forEach { runCatching { it.stopWatching() } }
        fileObservers.clear()
        networkCallback?.let { cb ->
            runCatching {
                val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
                cm.unregisterNetworkCallback(cb)
            }
        }
        scope.cancel()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }
}
