package com.obsync.viewmodel

import android.app.Application
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.obsync.bridge.RustBridge
import com.obsync.service.SyncService
import com.obsync.service.SyncServiceState
import com.obsync.service.SyncServiceStatus
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

data class SyncState(
    val vaultPath: String = "",
    val vaultName: String = "",
    val fileCount: Int = 0,
    val status: SyncStatus = SyncStatus.Idle,
    val deviceId: String = "",
    val deviceName: String = "Android",
    val fingerprint: String = "",
    val pairedDevices: List<PairedDevice> = emptyList(),
    val conflicts: List<ConflictEntry> = emptyList(),
    val snapshots: List<SnapshotEntry> = emptyList(),
    val recentFiles: List<FileEntry> = emptyList(),
    val peerAddress: String = "",
    val peerPort: Int = 42042,
    val pairedPeer: PairedPeer? = null,
    val syncing: Boolean = false,
    val lastSync: String = "",
    val error: String? = null,
)

data class PairedPeer(
    val host: String,
    val port: Int,
    val deviceName: String,
    val deviceId: String,
    val fingerprint: String,
)

enum class SyncStatus {
    Idle, Indexing, Discovering, Connecting, Syncing, Offline, Conflict, Error;

    val label: String
        get() = when (this) {
            Idle -> "Up to date"
            Indexing -> "Checking files"
            Discovering -> "Finding devices"
            Connecting -> "Connecting"
            Syncing -> "Syncing"
            Offline -> "Offline"
            Conflict -> "Conflict"
            Error -> "Error"
        }
}

data class PairedDevice(
    val deviceId: String,
    val deviceName: String,
    val fingerprint: String,
    val lastSeen: Long,
    val connected: Boolean,
)

data class ConflictEntry(
    val path: String,
    val localHash: String,
    val remoteHash: String,
    val detectedAt: Long,
)

data class SnapshotEntry(
    val path: String,
    val timestamp: Long,
    val size: Long,
)

data class FileEntry(
    val path: String,
    val size: Long,
    val modifiedAt: Long,
    val hashPrefix: String,
)

class SyncViewModel(application: Application) : AndroidViewModel(application) {
    private val _state = MutableStateFlow(SyncState())
    val state: StateFlow<SyncState> = _state.asStateFlow()

    init {
        RustBridge.ensureLoaded()
        loadPairedPeer()
        viewModelScope.launch { loadIdentity() }
        viewModelScope.launch {
            loadSavedVault()
            maybeAutoStart()
        }
        SyncService.state
            .onEach { s -> applyServiceState(s) }
            .launchIn(viewModelScope)
    }

    /** Start the background sync loop once both a vault and a peer are known. */
    private fun maybeAutoStart() {
        val s = _state.value
        if (s.vaultPath.isNotBlank() && s.peerAddress.isNotBlank() && !SyncService.isRunning()) {
            startSync()
        }
    }

    private fun applyServiceState(s: SyncServiceState) {
        _state.value = when (s.status) {
            SyncServiceStatus.Idle -> _state.value.copy(
                syncing = false,
                status = SyncStatus.Idle,
                lastSync = s.lastSync.ifEmpty { _state.value.lastSync },
            )
            SyncServiceStatus.Connecting -> _state.value.copy(syncing = true, status = SyncStatus.Connecting)
            SyncServiceStatus.Syncing -> _state.value.copy(syncing = true, status = SyncStatus.Syncing)
            SyncServiceStatus.Offline -> _state.value.copy(
                syncing = false,
                status = SyncStatus.Offline,
                lastSync = s.lastSync.ifEmpty { _state.value.lastSync },
            )
        }
        if (s.status != SyncServiceStatus.Syncing) {
            viewModelScope.launch {
                loadFiles()
                loadConflicts()
                loadSnapshots()
            }
        }
    }

    private fun prefs() =
        getApplication<Application>().getSharedPreferences("obsync", Context.MODE_PRIVATE)

    private fun loadPairedPeer() {
        val raw = prefs().getString("paired_peer", null) ?: return
        try {
            val json = JSONObject(raw)
            val device = PairedPeer(
                host = json.getString("host"),
                port = json.optInt("port", 42042),
                deviceName = json.optString("device_name", "Desktop"),
                deviceId = json.optString("device_id", ""),
                fingerprint = json.optString("fingerprint", ""),
            )
            _state.value = _state.value.copy(pairedPeer = device, peerAddress = device.host, peerPort = device.port)
        } catch (_: Exception) {}
    }

    private suspend fun loadSavedVault() {
        val path = prefs().getString("vault_path", null) ?: return
        try {
            val result = RustBridge.indexVault(path, buildIdentityJson())
            val json = JSONObject(result)
            if (json.has("error")) return
            _state.value = _state.value.copy(
                vaultPath = path,
                vaultName = path.split("/").lastOrNull() ?: "Vault",
                fileCount = json.optInt("file_count", 0),
                status = SyncStatus.Idle,
            )
            loadFiles()
            loadConflicts()
            loadSnapshots()
        } catch (_: Exception) {}
    }

    private fun savePairedPeer(device: PairedPeer) {
        val json = JSONObject().apply {
            put("host", device.host)
            put("port", device.port)
            put("device_name", device.deviceName)
            put("device_id", device.deviceId)
            put("fingerprint", device.fingerprint)
        }.toString()
        prefs().edit().putString("paired_peer", json).apply()
    }

    fun forgetPeer() {
        prefs().edit().remove("paired_peer").apply()
        _state.value = _state.value.copy(
            pairedPeer = null,
            peerAddress = "",
            lastSync = "Pairing removed",
        )
    }

    fun selectVault(uri: Uri) {
        viewModelScope.launch {
            _state.value = _state.value.copy(status = SyncStatus.Indexing)
            try {
                val path = resolveTreePath(uri) ?: throw IllegalStateException(
                    "Could not resolve folder path for ${uri}"
                )
                if (!hasAllFilesAccess()) {
                    _state.value = _state.value.copy(
                        status = SyncStatus.Error,
                        error = "Grant Obsync access to all files in Settings",
                    )
                    return@launch
                }
                val identityJson = buildIdentityJson()
                val result = RustBridge.indexVault(path, identityJson)
                val json = JSONObject(result)
                if (json.has("error")) {
                    _state.value = _state.value.copy(
                        status = SyncStatus.Error,
                        error = json.optString("error"),
                    )
                    return@launch
                }
                prefs().edit().putString("vault_path", path).apply()
                _state.value = _state.value.copy(
                    vaultPath = path,
                    vaultName = path.split("/").lastOrNull() ?: "Vault",
                    fileCount = json.optInt("file_count", 0),
                    status = SyncStatus.Idle,
                )
                loadFiles()
                loadConflicts()
                loadSnapshots()
                maybeAutoStart()
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    status = SyncStatus.Error,
                    error = e.message,
                )
            }
        }
    }

    fun hasAllFilesAccess(): Boolean = Environment.isExternalStorageManager()

    fun requestAllFilesAccess(): Intent =
        Intent(
            Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
            Uri.parse("package:${getApplication<Application>().packageName}")
        )

    private fun resolveTreePath(uri: Uri): String? {
        val app = getApplication<Application>()
        val docId = try {
            DocumentsContract.getTreeDocumentId(uri)
        } catch (_: Exception) {
            return null
        }
        val parts = docId.split(":", limit = 2)
        val volume = parts[0]
        val rest = if (parts.size > 1) parts[1] else ""
        return when {
            volume == "primary" -> {
                val base = Environment.getExternalStorageDirectory().absolutePath
                if (rest.isEmpty()) base else "$base/$rest"
            }
            else -> {
                // Secondary volume (SD card): match by volume label via DocumentsContract
                app.contentResolver.query(
                    uri.buildUpon()
                        .appendPath("root")
                        .build(),
                    arrayOf(DocumentsContract.Root.COLUMN_ROOT_ID, DocumentsContract.Root.COLUMN_DOCUMENT_ID),
                    null, null, null,
                )?.use { c ->
                    if (c.moveToFirst()) {
                        val rootId = c.getString(c.getColumnIndexOrThrow(DocumentsContract.Root.COLUMN_ROOT_ID))
                        val rootDoc = c.getString(c.getColumnIndexOrThrow(DocumentsContract.Root.COLUMN_DOCUMENT_ID))
                        resolveRootPath(rootId, rootDoc, rest)
                    } else null
                }
            }
        }
    }

    private fun resolveRootPath(rootId: String, rootDocId: String, rest: String): String? {
        val app = getApplication<Application>()
        val rootUri = DocumentsContract.buildDocumentUri(
            "com.android.externalstorage.documents", rootDocId
        )
        val file = app.contentResolver.query(
            rootUri,
            arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE),
            null, null, null,
        )?.use { c ->
            if (c.moveToFirst()) {
                val display = c.getString(c.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME))
                val mime = c.getString(c.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_MIME_TYPE))
                // Emulated storage roots are directories (DIR) — use primary/external mapping
                if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                    val external = Environment.getExternalStorageDirectory().absolutePath
                    val label = display.uppercase()
                    val parent = external.substringBeforeLast("/")
                    "$parent/$label" to label
                } else null to null
            } else null to null
        }
        val (base, label) = file ?: return null
        return if (rest.isEmpty()) base else "$base/$rest"
    }

    fun startPairing() {
        viewModelScope.launch {
            _state.value = _state.value.copy(status = SyncStatus.Discovering)
            // QR scanner triggered from UI; pairing payload generated via RustBridge
        }
    }

    fun processScannedQr(qrData: String) {
        viewModelScope.launch {
            val clean = qrData.trim()
            try {
                val json = JSONObject(clean)
                val host = json.optString("host", "")
                val deviceId = json.optString("device_id", "")
                if (host.isBlank() || deviceId.isBlank()) {
                    throw JSONException("not an Obsync pairing code (missing \"host\" or \"device_id\")")
                }
                val port = json.optInt("port", 42042)
                val device = PairedPeer(
                    host = host,
                    port = port,
                    deviceName = json.optString("device_name", "Desktop"),
                    deviceId = deviceId,
                    fingerprint = json.optString("public_key_fingerprint", ""),
                )
                savePairedPeer(device)
                _state.value = _state.value.copy(
                    pairedPeer = device,
                    peerAddress = host,
                    peerPort = port,
                    status = SyncStatus.Idle,
                    error = null,
                )
                maybeAutoStart()
            } catch (e: Exception) {
                val snippet = clean.take(80)
                _state.value = _state.value.copy(
                    error = "Pairing failed: ${e.message}. Scanned: \"$snippet\""
                )
            }
        }
    }

    fun refreshFiles() { viewModelScope.launch { loadFiles() } }

    fun setPeerAddress(addr: String) {
        val clean = addr.trim()
        _state.value = _state.value.copy(peerAddress = clean)
    }

    fun startSync() {
        val s = _state.value
        if (s.vaultPath.isBlank() || s.peerAddress.isBlank()) {
            _state.value = _state.value.copy(
                error = if (s.vaultPath.isBlank()) "Select a vault first" else "Enter the laptop's IP address"
            )
            return
        }
        _state.value = _state.value.copy(syncing = true, error = null, status = SyncStatus.Connecting)
        val ctx = getApplication<Application>()
        val intent = Intent(ctx, SyncService::class.java).apply {
            putExtra(SyncService.EXTRA_ADDR, s.peerAddress)
            putExtra(SyncService.EXTRA_PORT, s.peerPort)
            putExtra(SyncService.EXTRA_VAULT, s.vaultPath)
        }
        ctx.startForegroundService(intent)
        _state.value = _state.value.copy(
            syncing = true,
            status = SyncStatus.Syncing,
            lastSync = "Started sync with ${s.peerAddress}",
        )
    }

    fun stopSync() {
        val ctx = getApplication<Application>()
        ctx.stopService(Intent(ctx, SyncService::class.java))
        SyncService.state.value = SyncServiceState()
        _state.value = _state.value.copy(
            syncing = false,
            status = SyncStatus.Idle,
            lastSync = "Sync stopped",
        )
    }

    private suspend fun loadConflicts() {
        if (_state.value.vaultPath.isBlank()) return
        try {
            val result = RustBridge.listConflicts(_state.value.vaultPath, buildIdentityJson())
            val json = JSONObject(result)
            if (json.has("error")) return
            val arr = json.optJSONArray("conflicts") ?: JSONArray()
            val entries = (0 until arr.length()).map { i ->
                val c = arr.getJSONObject(i)
                ConflictEntry(
                    path = c.getString("path"),
                    localHash = c.optString("local_hash", ""),
                    remoteHash = c.optString("remote_hash", ""),
                    detectedAt = c.optLong("detected_at", 0L),
                )
            }
            _state.value = _state.value.copy(conflicts = entries)
        } catch (_: Exception) {}
    }

    fun resolveConflict(path: String, resolution: String) {
        viewModelScope.launch {
            if (_state.value.vaultPath.isBlank()) return@launch
            try {
                val result = RustBridge.resolveConflict(
                    _state.value.vaultPath, buildIdentityJson(), path, resolution
                )
                val json = JSONObject(result)
                if (json.has("error")) {
                    _state.value = _state.value.copy(error = json.optString("error"))
                    return@launch
                }
                _state.value = _state.value.copy(error = null)
                loadConflicts()
                loadSnapshots()
                loadFiles()
                maybeAutoStart()
            } catch (e: Exception) {
                _state.value = _state.value.copy(error = e.message)
            }
        }
    }

    private suspend fun loadSnapshots() {
        if (_state.value.vaultPath.isBlank()) return
        try {
            val result = RustBridge.listSnapshots(_state.value.vaultPath)
            val json = JSONObject(result)
            if (json.has("error")) return
            val arr = json.optJSONArray("snapshots") ?: JSONArray()
            val entries = (0 until arr.length()).map { i ->
                val s = arr.getJSONObject(i)
                SnapshotEntry(
                    path = s.getString("path"),
                    timestamp = s.getLong("timestamp"),
                    size = s.optLong("size", 0L),
                )
            }
            _state.value = _state.value.copy(snapshots = entries)
        } catch (_: Exception) {}
    }

    fun restoreSnapshot(path: String, timestamp: Long) {
        viewModelScope.launch {
            if (_state.value.vaultPath.isBlank()) return@launch
            try {
                val result = RustBridge.restoreSnapshot(
                    _state.value.vaultPath, buildIdentityJson(), path, timestamp
                )
                val json = JSONObject(result)
                if (json.has("error")) {
                    _state.value = _state.value.copy(error = json.optString("error"))
                    return@launch
                }
                _state.value = _state.value.copy(error = null)
                loadSnapshots()
                loadFiles()
                maybeAutoStart()
            } catch (e: Exception) {
                _state.value = _state.value.copy(error = e.message)
            }
        }
    }

    fun refreshConflicts() { viewModelScope.launch { loadConflicts() } }

    private suspend fun loadFiles() {
        try {
            val manifest = RustBridge.buildManifest(dbPath())
            val json = JSONObject(manifest)
            val files = json.optJSONArray("files") ?: JSONArray()
            val entries = (0 until files.length()).map { i ->
                val f = files.getJSONObject(i)
                FileEntry(
                    path = f.getString("relative_path"),
                    size = f.getLong("size"),
                    modifiedAt = f.getLong("modified_at"),
                    hashPrefix = f.optString("content_hash", "").take(8),
                )
            }
            _state.value = _state.value.copy(
                recentFiles = entries,
                fileCount = entries.size,
            )
        } catch (_: Exception) {}
    }

    private fun loadIdentity() {
        try {
            val data = RustBridge.loadOrCreateIdentity("Android", identityConfigPath())
            val json = JSONObject(String(data))
            _state.value = _state.value.copy(
                deviceId = json.optString("device_id", ""),
                deviceName = json.optString("device_name", "Android"),
                fingerprint = json.optString("fingerprint", ""),
            )
        } catch (_: Exception) {}
    }

    private fun identityConfigPath(): String =
        "${getApplication<Application>().filesDir.absolutePath}/identity.bin"

    private fun buildIdentityJson(): String = JSONObject().apply {
        put("device_id", _state.value.deviceId)
        put("device_name", _state.value.deviceName)
    }.toString()

    private fun dbPath(): String = "${_state.value.vaultPath}/.obsync/obsync.db"
}
