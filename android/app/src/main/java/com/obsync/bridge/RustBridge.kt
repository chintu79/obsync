package com.obsync.bridge

object RustBridge {
    private var loaded = false

    fun ensureLoaded() {
        if (!loaded) {
            System.loadLibrary("obsync_core")
            loaded = true
        }
    }

    // ── Identity ──
    external fun generateIdentity(name: String): ByteArray
    external fun loadOrCreateIdentity(name: String, configPath: String): ByteArray
    external fun identityFingerprint(data: ByteArray): String
    external fun identityDeviceId(data: ByteArray): String
    external fun identityDeviceName(data: ByteArray): String

    // ── Vault Indexing ──
    external fun indexVault(path: String, identityJson: String): String

    // ── Manifest ──
    external fun buildManifest(dbPath: String): String

    // ── Comparison ──
    external fun compareManifests(localJson: String, remoteJson: String): String

    // ── Sync Operations ──
    external fun applyOperation(dbPath: String, vaultPath: String, opJson: String): String

    // ── Conflicts ──
    external fun listConflicts(vaultPath: String, identityJson: String): String
    external fun resolveConflict(
        vaultPath: String,
        identityJson: String,
        relativePath: String,
        resolution: String,
    ): String

    // ── Version snapshots ──
    external fun listSnapshots(vaultPath: String): String
    external fun restoreSnapshot(
        vaultPath: String,
        identityJson: String,
        relativePath: String,
        timestamp: Long,
    ): String

    // ── Hashing ──
    external fun hashFile(path: String): String

    // ── Network ──
    external fun syncOnce(addr: String, port: Int, vaultPath: String, identityJson: String): String
    external fun connectPeer(addr: String, port: Int, identityJson: String): String
    external fun sendMessage(connJson: String, msgJson: String): String
    external fun receiveMessage(connJson: String): String

    // ── Pairing ──
    external fun generatePairingPayload(identityJson: String): String

    // ── Encryption ──
    external fun encrypt(data: ByteArray, keyHex: String): ByteArray
    external fun decrypt(data: ByteArray, keyHex: String): ByteArray
}
