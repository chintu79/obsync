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
import androidx.compose.ui.unit.sp
import com.obsync.viewmodel.SyncViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConflictsScreen(vm: SyncViewModel) {
    val state by vm.state.collectAsState()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Conflicts") },
                navigationIcon = {
                    IconButton(onClick = { /* nav handled by parent */ }) {
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
                        Text("All files are synchronized", fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            } else {
                Text("${state.conflicts.size} conflict(s) need resolution", style = MaterialTheme.typography.bodyMedium)
                state.conflicts.forEach { c ->
                    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp)) {
                        Column(Modifier.padding(16.dp)) {
                            Text(c.path, fontWeight = FontWeight.Medium, fontSize = 14.sp)
                            Spacer(Modifier.height(4.dp))
                            Text("Modified on both devices", fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.height(8.dp))
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                OutlinedButton(onClick = { /* keep local */ }, modifier = Modifier.weight(1f)) {
                                    Text("Keep Local", fontSize = 12.sp)
                                }
                                OutlinedButton(onClick = { /* keep remote */ }, modifier = Modifier.weight(1f)) {
                                    Text("Keep Remote", fontSize = 12.sp)
                                }
                                Button(onClick = { /* keep both */ }, modifier = Modifier.weight(1f)) {
                                    Text("Keep Both", fontSize = 12.sp)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
