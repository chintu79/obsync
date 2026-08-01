package com.obsync

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.obsync.service.SyncService
import com.obsync.ui.*
import com.obsync.viewmodel.SyncViewModel

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val viewModel = SyncViewModel(application)
        setContent {
            ObsyncTheme {
                Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    val nav = rememberNavController()
                    NavHost(nav, startDestination = "dashboard") {
                        composable("dashboard") { DashboardScreen(viewModel, nav) }
                        composable("pairing") { PairingScreen(viewModel, nav) }
                        composable("devices") { DevicesScreen(viewModel, nav) }
                        composable("conflicts") { ConflictsScreen(viewModel, nav) }
                        composable("settings") { SettingsScreen(viewModel, nav) }
                        composable("versions") { VersionsScreen(viewModel, nav) }
                    }
                }
            }
        }
    }

    override fun onStart() {
        super.onStart()
        // Returning to the app: sync immediately if the loop is running.
        SyncService.syncNow()
    }
}
