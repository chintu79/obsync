package com.obsync.ui

import android.Manifest
import android.content.pm.PackageManager
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.*
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.navigation.NavController
import android.util.Log
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.obsync.viewmodel.SyncViewModel
import java.util.concurrent.Executors

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairingScreen(vm: SyncViewModel, nav: NavController) {
    val ctx = LocalContext.current
    var hasCamera by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        )
    }
    var manualText by remember { mutableStateOf("") }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted -> hasCamera = granted }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Pair Device") },
                navigationIcon = { IconButton(onClick = { nav.popBackStack() }) { Icon(Icons.Default.ArrowBack, "Back") } }
            )
        }
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            if (!hasCamera) {
                Text("Camera permission required to scan QR code", fontSize = 14.sp)
                Spacer(Modifier.height(12.dp))
                Button(onClick = {
                    permissionLauncher.launch(Manifest.permission.CAMERA)
                }) { Text("Grant Camera Permission") }
            } else {
                Text("Point at the QR code on your desktop", fontSize = 14.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(16.dp))

                Box(
                    Modifier.size(280.dp),
                    contentAlignment = Alignment.Center
                ) {
                    QRScanner { data ->
                        vm.processScannedQr(data)
                        nav.popBackStack()
                    }
                }

                Spacer(Modifier.height(24.dp))
                Text("Or enter pairing code manually", fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = manualText,
                    onValueChange = { manualText = it },
                    placeholder = { Text("Paste QR data here") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                )
                Spacer(Modifier.height(8.dp))
                Button(
                    onClick = { vm.processScannedQr(manualText); nav.popBackStack() },
                    enabled = manualText.isNotBlank(),
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Pair") }
            }
        }
    }
}

@Composable
fun QRScanner(onScan: (String) -> Unit) {
    val ctx = LocalContext.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val reader = remember {
        MultiFormatReader().apply {
            setHints(mapOf(
                DecodeHintType.POSSIBLE_FORMATS to listOf(
                    BarcodeFormat.QR_CODE,
                    BarcodeFormat.DATA_MATRIX,
                    BarcodeFormat.AZTEC,
                    BarcodeFormat.CODE_128,
                    BarcodeFormat.CODE_39,
                ),
                DecodeHintType.TRY_HARDER to true,
            ))
        }
    }
    val scanning = remember { java.util.concurrent.atomic.AtomicBoolean(true) }

    DisposableEffect(Unit) {
        onDispose { executor.shutdown() }
    }

    AndroidView(
        factory = { context ->
            val previewView = PreviewView(context)
            val provider = ProcessCameraProvider.getInstance(context)

            provider.addListener({
                val cameraProvider = provider.get()
                val preview = Preview.Builder().build().also { it.setSurfaceProvider(previewView.surfaceProvider) }

                val analyzer = ImageAnalysis.Builder()
                    .setTargetResolution(Size(1280, 720))
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()

                analyzer.setAnalyzer(executor) { imageProxy ->
                    if (!scanning.get()) { imageProxy.close(); return@setAnalyzer }
                    val pixels = imageProxyToPixels(imageProxy)
                    if (pixels != null) {
                        try {
                            val degrees = imageProxy.imageInfo.rotationDegrees
                            val rotated = rotatePixels(pixels, imageProxy.width, imageProxy.height, degrees)
                            val w = if (degrees % 180 == 0) imageProxy.width else imageProxy.height
                            val h = if (degrees % 180 == 0) imageProxy.height else imageProxy.width

                            var result = tryDecode(reader, rotated, w, h)
                            if (result == null) {
                                // fallback: inverted luminance (dark-mode QR)
                                val inverted = invertedPixels(rotated)
                                result = tryDecode(reader, inverted, w, h)
                            }
                            result?.text?.let {
                                if (scanning.getAndSet(false)) {
                                    onScan(it)
                                }
                            }
                        } catch (_: Exception) {}
                    }
                    imageProxy.close()
                }

                cameraProvider.bindToLifecycle(
                    ctx as androidx.lifecycle.LifecycleOwner,
                    CameraSelector.DEFAULT_BACK_CAMERA,
                    preview, analyzer,
                )
            }, ContextCompat.getMainExecutor(context))

            previewView
        },
        modifier = Modifier.fillMaxSize(),
    )
}

private fun tryDecode(reader: MultiFormatReader, pixels: IntArray, w: Int, h: Int): com.google.zxing.Result? {
    return try {
        reader.decodeWithState(BinaryBitmap(HybridBinarizer(RGBLuminanceSource(w, h, pixels))))
    } catch (_: Exception) {
        null
    } finally {
        reader.reset()
    }
}

private fun invertedPixels(src: IntArray): IntArray {
    val out = IntArray(src.size)
    for (i in src.indices) {
        val v = src[i] and 0xFF
        out[i] = (0xFF shl 24) or (v.inv() and 0xFF shl 16) or (v.inv() and 0xFF shl 8) or (v.inv() and 0xFF)
    }
    return out
}

private fun imageProxyToPixels(image: ImageProxy): IntArray? {
    val buffer = image.planes[0].buffer ?: return null
    val width = image.width
    val height = image.height
    val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
    val pixels = IntArray(width * height)
    // Convert YUV luminance to gray ARGB for ZXing, respecting row stride
    val rowStride = image.planes[0].rowStride
    for (y in 0 until height) {
        val rowStart = y * rowStride
        for (x in 0 until width) {
            val i = rowStart + x
            if (i >= bytes.size) break
            val v = bytes[i].toInt() and 0xFF
            pixels[y * width + x] = (0xFF shl 24) or (v shl 16) or (v shl 8) or v
        }
    }
    return pixels
}

private fun rotatePixels(src: IntArray, width: Int, height: Int, degrees: Int): IntArray {
    if (degrees == 0) return src
    val out = IntArray(src.size)
    when (degrees % 360) {
        90 -> for (y in 0 until height) for (x in 0 until width) out[x * height + (height - 1 - y)] = src[y * width + x]
        180 -> for (y in 0 until height) for (x in 0 until width) out[(height - 1 - y) * width + (width - 1 - x)] = src[y * width + x]
        270 -> for (y in 0 until height) for (x in 0 until width) out[(width - 1 - x) * height + y] = src[y * width + x]
    }
    return out
}
