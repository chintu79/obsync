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
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.navigation.NavController
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
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
                    IconButton(onClick = { nav.navigate("settings") { launchSingleTop = true } }) {
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
                        TextButton(onClick = { vm.refreshFiles() }) { Text("Refresh") }
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
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
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
            Text("Choose your Obsidian vault folder", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(16.dp))
            Button(onClick = onPick) { Text("Browse vault") }
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
                Text(state.vaultName, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text("${state.fileCount} files · ${state.status.label}", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (state.fileCount > 0) {
                Text("${state.fileCount}", style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"), fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
                        Text(state.pairedPeer.deviceName, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                        Text(
                            if (state.syncing) "Syncing…" else "${state.pairedPeer.host}:${state.pairedPeer.port}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (state.syncing) {
                        Button(onClick = { vm.stopSync() }) { Text("Stop sync") }
                    } else {
                        Button(onClick = { vm.startSync() }) { Text("Sync now") }
                    }
                    TextButton(onClick = { vm.forgetPeer() }) { Text("Unpair") }
                }
                if (state.lastSync.isNotEmpty()) {
                    Spacer(Modifier.height(4.dp))
                    Text(state.lastSync, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            } else {
                Text("Scan the QR code shown on the laptop to pair once — then sync anytime.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(10.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedButton(onClick = { nav.navigate("pairing") { launchSingleTop = true } }) { Text("Scan QR code") }
                    Text("or", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
                        Button(onClick = { vm.stopSync() }) { Text("Stop sync") }
                    } else {
                        Button(onClick = { vm.startSync() }) { Text("Sync now") }
                    }
                    if (state.lastSync.isNotEmpty()) {
                        Text(state.lastSync, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            state.error?.let {
                Spacer(Modifier.height(8.dp))
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                )
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
                TextButton(onClick = { nav.navigate("devices") { launchSingleTop = true } }) { Text("Manage") }
            }
            if (state.pairedDevices.isEmpty()) {
                Text("No paired devices", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(8.dp))
                OutlinedButton(onClick = { nav.navigate("pairing") { launchSingleTop = true } }) { Text("Pair device") }
            } else {
                state.pairedDevices.forEach { d ->
                    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.size(8.dp).clip(CircleShape).background(if (d.connected) Color(0xFF16A34A) else Color(0xFF737373)))
                        Spacer(Modifier.width(8.dp))
                        Text(d.deviceName, style = MaterialTheme.typography.titleSmall)
                        Spacer(Modifier.weight(1f))
                        Text(if (d.connected) "Connected" else "Offline", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
        Text(path, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Text(
            when { size < 1024 -> "$size B"; size < 1048576 -> "${size / 1024} KB"; else -> "${size / 1048576} MB" },
            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
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
    val currentBackStackEntry by nav.currentBackStackEntryAsState()
    val currentRoute = currentBackStackEntry?.destination?.route
    NavigationBar {
        NavigationBarItem(
            selected = currentRoute == "dashboard",
            onClick = { nav.navigate("dashboard") { popUpTo(nav.graph.startDestinationId) { saveState = true }; launchSingleTop = true; restoreState = true } },
            icon = { Icon(Icons.Default.Home, null) }, label = { Text("Home") })
        NavigationBarItem(
            selected = currentRoute == "devices",
            onClick = { nav.navigate("devices") { launchSingleTop = true } },
            icon = { Icon(Icons.Default.Devices, null) }, label = { Text("Devices") })
        NavigationBarItem(
            selected = currentRoute == "conflicts",
            onClick = { nav.navigate("conflicts") { launchSingleTop = true } },
            icon = { Icon(Icons.Default.Warning, null) }, label = { Text("Conflicts") })
    }
}
