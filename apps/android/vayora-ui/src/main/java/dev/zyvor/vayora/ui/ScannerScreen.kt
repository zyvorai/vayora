package dev.zyvor.vayora.ui

import android.Manifest
import android.content.pm.PackageManager
import android.util.Size
import android.view.ViewGroup
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.FlashlightOff
import androidx.compose.material.icons.filled.FlashlightOn
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size as GeomSize
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.min

/**
 * Camera QR scanner with a dimmed viewfinder, matching iOS `ScannerView`.
 * Hands the raw payload to [onCode] once; caller parses (e.g. `UPIPayload`).
 */
@Composable
fun ScannerScreen(
    brand: Brand = Brand.Pay,
    hint: String = "Aim at a QR code",
    onCode: (String) -> Unit,
    onCancel: () -> Unit,
) {
    val ctx = LocalContext.current
    var permitted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED,
        )
    }
    var denied by remember { mutableStateOf(false) }
    var locked by remember { mutableStateOf(false) }
    var torchOn by remember { mutableStateOf(false) }
    var camera by remember { mutableStateOf<Camera?>(null) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        permitted = ok
        denied = !ok
    }
    LaunchedEffect(Unit) {
        if (!permitted) ask.launch(Manifest.permission.CAMERA)
    }
    LaunchedEffect(torchOn, camera) {
        camera?.cameraControl?.enableTorch(torchOn)
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        if (permitted) {
            CameraPreview(
                onCamera = { camera = it },
                onCode = { text ->
                    if (!locked) {
                        locked = true
                        onCode(text)
                    }
                },
            )
        }
        ViewfinderOverlay(locked = locked, accent = if (locked) Vui.Mint else Color.White)
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(
                    onClick = onCancel,
                    modifier = Modifier
                        .size(40.dp)
                        .background(Color.Black.copy(alpha = 0.4f), CircleShape)
                        .semantics { contentDescription = "Close scanner" },
                ) {
                    Icon(Icons.Filled.Close, null, tint = Color.White)
                }
                Spacer(Modifier.weight(1f))
                IconButton(
                    onClick = { torchOn = !torchOn },
                    modifier = Modifier
                        .size(40.dp)
                        .background(Color.Black.copy(alpha = 0.4f), CircleShape)
                        .semantics {
                            contentDescription = if (torchOn) "Turn the torch off" else "Turn the torch on"
                        },
                ) {
                    Icon(
                        if (torchOn) Icons.Filled.FlashlightOn else Icons.Filled.FlashlightOff,
                        null,
                        tint = if (torchOn) Vui.Amber else Color.White,
                    )
                }
            }
            Spacer(Modifier.weight(1f))
            Text(
                if (locked) "Got it" else hint,
                color = Color.White,
                fontSize = 17.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier
                    .align(Alignment.CenterHorizontally)
                    .padding(bottom = 56.dp)
                    .background(Color.Black.copy(alpha = 0.45f), RoundedCornerShape(999.dp))
                    .padding(horizontal = 18.dp, vertical = 10.dp),
            )
        }
        if (denied) {
            Text(
                "Camera access is off. Allow it in Settings, then try again.",
                color = Color.White,
                fontSize = 15.sp,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .align(Alignment.Center)
                    .padding(30.dp)
                    .background(Color.Black.copy(alpha = 0.7f), RoundedCornerShape(18.dp))
                    .padding(20.dp),
            )
        }
        Box(
            Modifier
                .align(Alignment.TopCenter)
                .padding(top = 64.dp)
                .size(8.dp)
                .background(brand.primary, CircleShape),
        )
    }
}

@Composable
private fun ViewfinderOverlay(locked: Boolean, accent: Color) {
    Canvas(Modifier.fillMaxSize()) {
        val side = min(size.width * 0.68f, 290.dp.toPx())
        val left = (size.width - side) / 2f
        val top = (size.height - side) / 2f - 30.dp.toPx()
        val dim = Path().apply {
            fillType = PathFillType.EvenOdd
            addRect(Rect(Offset.Zero, GeomSize(size.width, size.height)))
            addRoundRect(RoundRect(Rect(left, top, left + side, top + side), CornerRadius(26.dp.toPx())))
        }
        drawPath(dim, Color.Black.copy(alpha = 0.55f))
        val arm = min(38.dp.toPx(), side * 0.22f)
        val stroke = Stroke(width = 5.dp.toPx(), cap = StrokeCap.Round)
        fun corner(x: Float, y: Float, dx: Float, dy: Float) {
            drawLine(accent, Offset(x + dx * arm, y), Offset(x, y), stroke.width, StrokeCap.Round)
            drawLine(accent, Offset(x, y), Offset(x, y + dy * arm), stroke.width, StrokeCap.Round)
        }
        corner(left, top, 1f, 1f)
        corner(left + side, top, -1f, 1f)
        corner(left, top + side, 1f, -1f)
        corner(left + side, top + side, -1f, -1f)
        if (locked) {
            drawRoundRect(
                color = accent.copy(alpha = 0.35f),
                topLeft = Offset(left, top),
                size = GeomSize(side, side),
                cornerRadius = CornerRadius(26.dp.toPx()),
                style = Stroke(width = 2.dp.toPx()),
            )
        }
    }
}

@Composable
private fun CameraPreview(
    onCamera: (Camera) -> Unit,
    onCode: (String) -> Unit,
) {
    val ctx = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val done = remember { AtomicBoolean(false) }
    val reader = remember {
        MultiFormatReader().apply {
            setHints(
                mapOf(
                    DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
                    DecodeHintType.TRY_HARDER to true,
                ),
            )
        }
    }
    val previewView = remember {
        PreviewView(ctx).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            scaleType = PreviewView.ScaleType.FILL_CENTER
            implementationMode = PreviewView.ImplementationMode.COMPATIBLE
        }
    }

    DisposableEffect(lifecycle) {
        val providerFuture = ProcessCameraProvider.getInstance(ctx)
        val main = ContextCompat.getMainExecutor(ctx)
        val listener = Runnable {
            val provider = providerFuture.get()
            val preview = Preview.Builder().build().also {
                it.surfaceProvider = previewView.surfaceProvider
            }
            val analysis = ImageAnalysis.Builder()
                .setTargetResolution(Size(1280, 720))
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .also { analysis ->
                    analysis.setAnalyzer(executor) { image ->
                        if (done.get()) {
                            image.close()
                            return@setAnalyzer
                        }
                        decodeQr(image, reader)?.let { text ->
                            if (done.compareAndSet(false, true)) {
                                previewView.post { onCode(text) }
                            }
                        }
                        image.close()
                    }
                }
            provider.unbindAll()
            val cam = provider.bindToLifecycle(
                lifecycle,
                CameraSelector.DEFAULT_BACK_CAMERA,
                preview,
                analysis,
            )
            onCamera(cam)
        }
        providerFuture.addListener(listener, main)
        onDispose {
            runCatching {
                ProcessCameraProvider.getInstance(ctx).get().unbindAll()
            }
            executor.shutdown()
        }
    }

    AndroidView(factory = { previewView }, modifier = Modifier.fillMaxSize())
}

private fun decodeQr(image: ImageProxy, reader: MultiFormatReader): String? {
    return try {
        val plane = image.planes[0]
        val buffer = plane.buffer
        val data = ByteArray(buffer.remaining())
        buffer.get(data)
        val width = image.width
        val height = image.height
        val source = PlanarYUVLuminanceSource(data, width, height, 0, 0, width, height, false)
        val bitmap = BinaryBitmap(HybridBinarizer(source))
        reader.decodeWithState(bitmap)?.text
    } catch (_: Exception) {
        null
    } finally {
        reader.reset()
    }
}
