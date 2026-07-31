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
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONArray
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

enum class SyncStatus { Idle, Indexing, Discovering, Connecting, Syncing, Offline, Conflict, Error }

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
        viewModelScope.launch { loadSavedVault() }
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
            try {
                val result = RustBridge.parsePairingPayload(qrData)
                val json = JSONObject(result)
                val host = json.optString("host", "")
                val port = json.optInt("port", 42042)
                val device = PairedPeer(
                    host = host,
                    port = port,
                    deviceName = json.optString("device_name", "Desktop"),
                    deviceId = json.optString("device_id", ""),
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
            } catch (e: Exception) {
                _state.value = _state.value.copy(error = "Pairing failed: ${e.message}")
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
        _state.value = _state.value.copy(
            syncing = false,
            status = SyncStatus.Idle,
            lastSync = "Sync stopped",
        )
    }

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
