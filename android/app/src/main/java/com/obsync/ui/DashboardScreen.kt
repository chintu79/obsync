package com.obsync.ui

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.navigation.NavController
import com.obsync.viewmodel.SyncState
import com.obsync.viewmodel.SyncStatus
import com.obsync.viewmodel.SyncViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DashboardScreen(vm: SyncViewModel, nav: NavController) {
    val state by vm.state.collectAsState()
    val picker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree()
    ) { uri: Uri? -> uri?.let { vm.selectVault(it) } }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Obsync", fontWeight = FontWeight.SemiBold) },
                actions = {
                    IconButton(onClick = { nav.navigate("settings") }) {
                        Icon(Icons.Default.Settings, "Settings")
                    }
                }
            )
        },
        bottomBar = { BottomBar(nav) }
    ) { pad ->
        LazyColumn(modifier = Modifier.fillMaxSize().padding(pad).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (!vm.hasAllFilesAccess()) {
                item { FileAccessPrompt(vm) }
            }
            if (state.vaultPath.isEmpty()) {
                item { NoVaultPrompt { picker.launch(null) } }
            } else {
                item { VaultCard(state) }
                item { PeerCard(vm, state, nav) }
                item { DeviceCard(state, nav) }
                item {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("Files", style = MaterialTheme.typography.titleSmall)
                        TextButton(onClick = { vm.refreshFiles() }) { Text("Refresh", fontSize = 12.sp) }
                    }
                }
                items(state.recentFiles.take(10)) { f ->
                    FileRow(f.path, f.size)
                }
            }
        }
    }
}

@Composable
fun FileAccessPrompt(vm: SyncViewModel) {
    val ctx = LocalContext.current
    val launcher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { _ -> }
    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
        Column(Modifier.padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(Icons.Default.Lock, null, modifier = Modifier.size(40.dp), tint = MaterialTheme.colorScheme.error)
            Spacer(Modifier.height(12.dp))
            Text("File access required", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(4.dp))
            Text(
                "Obsync needs access to all files so it can sync your vault folder.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 13.sp
            )
            Spacer(Modifier.height(16.dp))
            Button(onClick = {
                try {
                    launcher.launch(vm.requestAllFilesAccess())
                } catch (_: Exception) {
                    ctx.startActivity(android.content.Intent(android.provider.Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
                }
            }) { Text("Grant file access") }
        }
    }
}

@Composable
fun NoVaultPrompt(onPick: () -> Unit) {
    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
        Column(Modifier.padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(Icons.Default.FolderOpen, null, modifier = Modifier.size(40.dp), tint = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.height(12.dp))
            Text("Select a vault directory", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(4.dp))
            Text("Choose your Obsidian vault folder", color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp)
            Spacer(Modifier.height(16.dp))
            Button(onClick = onPick) { Text("Browse Vault") }
        }
    }
}

@Composable
fun VaultCard(state: SyncState) {
    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(10.dp).clip(CircleShape).background(statusColor(state.status)))
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(state.vaultName, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
                Text("${state.fileCount} files · ${state.status.name}", color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp)
            }
            if (state.fileCount > 0) {
                Text("${state.fileCount}", fontWeight = FontWeight.SemiBold, fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
fun PeerCard(vm: SyncViewModel, state: SyncState, nav: NavController) {
    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
        Column(Modifier.padding(16.dp)) {
            Text("Sync with laptop", style = MaterialTheme.typography.titleSmall)
            Spacer(Modifier.height(4.dp))

            if (state.pairedPeer != null) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(10.dp).clip(CircleShape).background(if (state.syncing) Color(0xFF16A34A) else Color(0xFF737373)))
                    Spacer(Modifier.width(8.dp))
                    Column(Modifier.weight(1f)) {
                        Text(state.pairedPeer.deviceName, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                        Text("${state.pairedPeer.host}:${state.pairedPeer.port}", fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (state.syncing) {
                        Button(onClick = { vm.stopSync() }) { Text("Stop Sync") }
                    } else {
                        Button(onClick = { vm.startSync() }) { Text("Sync Now") }
                    }
                    TextButton(onClick = { vm.forgetPeer() }) { Text("Forget", fontSize = 12.sp) }
                }
            } else {
                Text("Scan the QR code shown on the laptop to pair once — then sync anytime.",
                    color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 12.sp)
                Spacer(Modifier.height(10.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedButton(onClick = { nav.navigate("pairing") }) { Text("Scan QR Code") }
                    Text("or", fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    OutlinedTextField(
                        value = state.peerAddress,
                        onValueChange = { vm.setPeerAddress(it) },
                        label = { Text("IP (manual)") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        modifier = Modifier.weight(1f),
                    )
                }
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (state.syncing) {
                        Button(onClick = { vm.stopSync() }) { Text("Stop Sync") }
                    } else {
                        Button(onClick = { vm.startSync() }) { Text("Sync Now") }
                    }
                    if (state.lastSync.isNotEmpty()) {
                        Text(state.lastSync, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            state.error?.let {
                Spacer(Modifier.height(8.dp))
                Text(it, fontSize = 12.sp, color = MaterialTheme.colorScheme.error)
            }
        }
    }
}

@Composable
fun DeviceCard(state: SyncState, nav: NavController) {
    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
        Column(Modifier.padding(16.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text("Devices", style = MaterialTheme.typography.titleSmall)
                TextButton(onClick = { nav.navigate("devices") }) { Text("Manage", fontSize = 12.sp) }
            }
            if (state.pairedDevices.isEmpty()) {
                Text("No paired devices", color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp)
                Spacer(Modifier.height(8.dp))
                OutlinedButton(onClick = { nav.navigate("pairing") }) { Text("Pair Device") }
            } else {
                state.pairedDevices.forEach { d ->
                    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.size(8.dp).clip(CircleShape).background(if (d.connected) Color(0xFF16A34A) else Color(0xFF737373)))
                        Spacer(Modifier.width(8.dp))
                        Text(d.deviceName, fontSize = 14.sp)
                        Spacer(Modifier.weight(1f))
                        Text(if (d.connected) "Connected" else "Offline", fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
    }
}

@Composable
fun FileRow(path: String, size: Long) {
    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.Default.Description, null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.width(8.dp))
        Text(path, fontSize = 13.sp, modifier = Modifier.weight(1f))
        Text(
            when { size < 1024 -> "$size B"; size < 1048576 -> "${size / 1024} KB"; else -> "${size / 1048576} MB" },
            fontSize = 12.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

fun statusColor(s: SyncStatus) = when (s) {
    SyncStatus.Idle -> Color(0xFF16A34A)
    SyncStatus.Indexing, SyncStatus.Syncing, SyncStatus.Discovering, SyncStatus.Connecting -> Color(0xFFD97706)
    SyncStatus.Offline -> Color(0xFF737373)
    SyncStatus.Conflict -> Color(0xFFDC2626)
    SyncStatus.Error -> Color(0xFFDC2626)
}

@Composable
fun BottomBar(nav: NavController) {
    NavigationBar {
        NavigationBarItem(selected = true, onClick = { nav.navigate("dashboard") },
            icon = { Icon(Icons.Default.Home, null) }, label = { Text("Home") })
        NavigationBarItem(selected = false, onClick = { nav.navigate("devices") },
            icon = { Icon(Icons.Default.Devices, null) }, label = { Text("Devices") })
        NavigationBarItem(selected = false, onClick = { nav.navigate("conflicts") },
            icon = { Icon(Icons.Default.Warning, null) }, label = { Text("Conflicts") })
    }
}
