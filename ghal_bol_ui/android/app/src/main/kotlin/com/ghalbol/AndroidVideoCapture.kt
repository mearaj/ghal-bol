package com.ghalbol

import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.media.Image
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Camera2 capture in the `:p2p` process — I420 frames pushed to Rust via JNI.
 * All camera work runs on one handler thread so stop→start never races.
 */
object AndroidVideoCapture {
    private const val TAG = "GhalBolVideo"
    private const val WIDTH = 640
    private const val HEIGHT = 480

    private val running = AtomicBoolean(false)
    private var cameraThread: HandlerThread? = null
    private var cameraHandler: Handler? = null
    private var imageReader: ImageReader? = null
    private var cameraDevice: CameraDevice? = null
    private var captureSession: CameraCaptureSession? = null

    @JvmStatic
    fun start(context: Context) {
        ensureThread()
        cameraHandler?.post {
            try {
                stopInternalLocked()
                running.set(true)
                openCamera(context.applicationContext)
            } catch (e: Throwable) {
                Log.w(TAG, "start failed: ${e.message}")
                stopInternalLocked()
                running.set(false)
            }
        }
    }

    @JvmStatic
    fun stop() {
        running.set(false)
        cameraHandler?.post { stopInternalLocked() }
    }

    private fun ensureThread() {
        if (cameraThread?.isAlive == true && cameraHandler != null) return
        val thread = HandlerThread("ghal_bol-camera2").also { it.start() }
        cameraThread = thread
        cameraHandler = Handler(thread.looper)
    }

    private fun stopInternalLocked() {
        try {
            captureSession?.close()
        } catch (_: Throwable) {
        }
        captureSession = null
        try {
            cameraDevice?.close()
        } catch (_: Throwable) {
        }
        cameraDevice = null
        try {
            imageReader?.close()
        } catch (_: Throwable) {
        }
        imageReader = null
    }

    private fun openCamera(context: Context) {
        if (!running.get()) return
        val mgr = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val cameraId =
            mgr.cameraIdList.firstOrNull { id ->
                val chars = mgr.getCameraCharacteristics(id)
                chars.get(CameraCharacteristics.LENS_FACING) ==
                    CameraCharacteristics.LENS_FACING_FRONT
            } ?: mgr.cameraIdList.firstOrNull()
        if (cameraId == null) {
            Log.w(TAG, "no camera available")
            running.set(false)
            return
        }
        val reader =
            ImageReader.newInstance(WIDTH, HEIGHT, ImageFormat.YUV_420_888, 3).also { ir ->
                ir.setOnImageAvailableListener({ r ->
                    if (!running.get()) return@setOnImageAvailableListener
                    val image = r.acquireLatestImage() ?: return@setOnImageAvailableListener
                    try {
                        deliver(image)
                    } catch (e: Throwable) {
                        Log.w(TAG, "deliver: ${e.message}")
                    } finally {
                        image.close()
                    }
                }, cameraHandler)
            }
        imageReader = reader
        mgr.openCamera(
            cameraId,
            object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    if (!running.get()) {
                        camera.close()
                        return
                    }
                    cameraDevice = camera
                    try {
                        camera.createCaptureSession(
                            listOf(reader.surface),
                            object : CameraCaptureSession.StateCallback() {
                                override fun onConfigured(session: CameraCaptureSession) {
                                    if (!running.get()) return
                                    captureSession = session
                                    try {
                                        val req =
                                            camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD)
                                                .apply { addTarget(reader.surface) }
                                                .build()
                                        session.setRepeatingRequest(req, null, cameraHandler)
                                        Log.i(TAG, "camera capture started ${WIDTH}x$HEIGHT")
                                    } catch (e: Throwable) {
                                        Log.w(TAG, "repeating request: ${e.message}")
                                        running.set(false)
                                        stopInternalLocked()
                                    }
                                }

                                override fun onConfigureFailed(session: CameraCaptureSession) {
                                    Log.w(TAG, "capture session configure failed")
                                    running.set(false)
                                    stopInternalLocked()
                                }
                            },
                            cameraHandler,
                        )
                    } catch (e: Throwable) {
                        Log.w(TAG, "createCaptureSession: ${e.message}")
                        running.set(false)
                        stopInternalLocked()
                    }
                }

                override fun onDisconnected(camera: CameraDevice) {
                    camera.close()
                    running.set(false)
                    stopInternalLocked()
                }

                override fun onError(camera: CameraDevice, error: Int) {
                    Log.w(TAG, "camera error=$error")
                    camera.close()
                    running.set(false)
                    stopInternalLocked()
                }
            },
            cameraHandler,
        )
    }

    private fun deliver(image: Image) {
        val w = image.width and 0x7FFFFFFE.toInt()
        val h = image.height and 0x7FFFFFFE.toInt()
        if (w <= 0 || h <= 0) return
        val i420 = yuv420888ToI420(image, w, h) ?: return
        P2pDaemonNative.pushCameraFrame(i420, w, h)
    }

    private fun yuv420888ToI420(image: Image, width: Int, height: Int): ByteArray? {
        val planes = image.planes
        if (planes.size < 3) return null
        val ySize = width * height
        val uvSize = (width / 2) * (height / 2)
        val out = ByteArray(ySize + 2 * uvSize)
        copyPlane(planes[0], width, height, out, 0)
        copyPlane(planes[1], width / 2, height / 2, out, ySize)
        copyPlane(planes[2], width / 2, height / 2, out, ySize + uvSize)
        return out
    }

    private fun copyPlane(
        plane: Image.Plane,
        width: Int,
        height: Int,
        out: ByteArray,
        offset: Int,
    ) {
        val buffer = plane.buffer
        val rowStride = plane.rowStride
        val ps = plane.pixelStride.coerceAtLeast(1)
        var dst = offset
        for (row in 0 until height) {
            var src = row * rowStride
            for (col in 0 until width) {
                out[dst++] = buffer.get(src)
                src += ps
            }
        }
    }
}
