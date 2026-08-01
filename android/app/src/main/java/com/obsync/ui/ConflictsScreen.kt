package com.obsync.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.obsync.viewmodel.SyncViewModel

@Composable
fun ConflictActions(vm: SyncViewModel, path: String) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        if (maxWidth < 340.dp) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                ConflictButton("Keep local", filled = false, Modifier.fillMaxWidth()) {
                    vm.resolveConflict(path, "KeepLocal")
                }
                ConflictButton("Keep remote", filled = false, Modifier.fillMaxWidth()) {
                    vm.resolveConflict(path, "KeepRemote")
                }
                ConflictButton("Keep both", filled = true, Modifier.fillMaxWidth()) {
                    vm.resolveConflict(path, "KeepBoth")
                }
            }
        } else {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                ConflictButton("Keep local", filled = false, Modifier.weight(1f)) {
                    vm.resolveConflict(path, "KeepLocal")
                }
                ConflictButton("Keep remote", filled = false, Modifier.weight(1f)) {
                    vm.resolveConflict(path, "KeepRemote")
                }
                ConflictButton("Keep both", filled = true, Modifier.weight(1f)) {
                    vm.resolveConflict(path, "KeepBoth")
                }
            }
        }
    }
}

@Composable
private fun ConflictButton(
    label: String,
    filled: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    if (filled) {
        Button(onClick = onClick, modifier = modifier) { Text(label) }
    } else {
        OutlinedButton(onClick = onClick, modifier = modifier) { Text(label) }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConflictsScreen(vm: SyncViewModel, nav: NavController) {
    val state by vm.state.collectAsState()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Conflicts") },
                navigationIcon = {
                    IconButton(onClick = { nav.popBackStack() }) {
                        Icon(Icons.Default.ArrowBack, "Back")
                    }
                }
            )
        }
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (state.conflicts.isEmpty()) {
                Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
                    Column(Modifier.padding(32.dp).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(Icons.Default.Warning, null, modifier = Modifier.size(40.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.height(12.dp))
                        Text("No conflicts", fontWeight = FontWeight.Medium)
                        Text("All files are synchronized", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            } else {
                Text(
                    if (state.conflicts.size == 1) "1 conflict needs resolution"
                    else "${state.conflicts.size} conflicts need resolution",
                    style = MaterialTheme.typography.bodyMedium
                )
                state.conflicts.forEach { c ->
                    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
                        Column(Modifier.padding(16.dp)) {
                            Text(c.path, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Medium)
                            Spacer(Modifier.height(4.dp))
                            Text("Modified on both devices", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.height(8.dp))
                            ConflictActions(vm, c.path)
                        }
                    }
                }
            }
        }
    }
}
