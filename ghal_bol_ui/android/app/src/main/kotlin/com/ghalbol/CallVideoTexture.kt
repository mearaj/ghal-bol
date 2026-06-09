package com.ghalbol

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import io.flutter.view.TextureRegistry
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Reads cross-process RGBA shm from `:p2p` and draws into a Flutter [Texture]
 * via [TextureRegistry.SurfaceProducer] — no base64 or Dart pixel work.
 */
object CallVideoTexture {
    private const val SHM_HEADER_SIZE = 32
    private val MAGIC = byteArrayOf('G'.code.toByte(), 'B'.code.toByte(), 'V'.code.toByte(), '1'.code.toByte())

    private data class Entry(
        val producer: TextureRegistry.SurfaceProducer,
        val thread: HandlerThread,
        val handler: Handler,
        val shmPath: String,
        var lastGeneration: Long = 0L,
        var width: Int = 0,
        var height: Int = 0,
        val paint: Paint = Paint(Paint.FILTER_BITMAP_FLAG),
        val pollRunnable: Runnable,
    )

    private val entries = mutableMapOf<Long, Entry>()

    fun register(registry: TextureRegistry, shmPath: String, width: Int, height: Int): Long {
        val producer = registry.createSurfaceProducer()
        val id = producer.id()
        val thread = HandlerThread("ghalbol_video_tex_$id").apply { start() }
        val handler = Handler(thread.looper)
        val poll =
            object : Runnable {
                override fun run() {
                    val e = entries[id] ?: return
                    drawIfNew(e)
                    e.handler.postDelayed(this, 16L)
                }
            }
        val entry =
            Entry(
                producer = producer,
                thread = thread,
                handler = handler,
                shmPath = shmPath,
                width = width.coerceAtLeast(1),
                height = height.coerceAtLeast(1),
                pollRunnable = poll,
            )
        producer.setCallback(
            object : TextureRegistry.SurfaceProducer.Callback {
                override fun onSurfaceAvailable() {
                    entries[id]?.let { drawIfNew(it) }
                }

                override fun onSurfaceCleanup() {
                    // Stop using the surface until onSurfaceAvailable.
                }
            },
        )
        producer.setSize(entry.width, entry.height)
        entries[id] = entry
        handler.post(poll)
        return id
    }

    fun release(textureId: Long) {
        val e = entries.remove(textureId) ?: return
        e.handler.removeCallbacks(e.pollRunnable)
        e.thread.quitSafely()
        e.producer.release()
    }

    fun releaseAll() {
        entries.keys.toList().forEach { release(it) }
    }

    private fun drawIfNew(entry: Entry): Boolean {
        val hdr = readHeader(entry.shmPath) ?: return false
        if (hdr.generation <= entry.lastGeneration) {
            return false
        }
        val rgba = readRgba(entry.shmPath, hdr.width, hdr.height) ?: return false
        entry.lastGeneration = hdr.generation
        if (entry.width != hdr.width || entry.height != hdr.height) {
            entry.width = hdr.width
            entry.height = hdr.height
            entry.producer.setSize(hdr.width, hdr.height)
        }
        val surface: Surface = entry.producer.surface ?: return true
        val bitmap =
            Bitmap.createBitmap(hdr.width, hdr.height, Bitmap.Config.ARGB_8888).apply {
                copyPixelsFromBuffer(ByteBuffer.wrap(rgba))
            }
        try {
            val canvas = surface.lockCanvas(null) ?: return true
            try {
                if (canvas.width == hdr.width && canvas.height == hdr.height) {
                    canvas.drawBitmap(bitmap, 0f, 0f, null)
                } else {
                    val dst = Rect(0, 0, canvas.width, canvas.height)
                    canvas.drawBitmap(bitmap, null, dst, entry.paint)
                }
            } finally {
                surface.unlockCanvasAndPost(canvas)
            }
        } finally {
            bitmap.recycle()
        }
        return true
    }

    private data class Header(val width: Int, val height: Int, val generation: Long)

    private fun readHeader(path: String): Header? {
        val file = File(path)
        if (!file.isFile || file.length() < SHM_HEADER_SIZE) return null
        RandomAccessFile(file, "r").use { raf ->
            val magic = ByteArray(4)
            if (raf.read(magic) != 4 || !magic.contentEquals(MAGIC)) return null
            val buf = ByteArray(16)
            if (raf.read(buf) != 16) return null
            val bb = ByteBuffer.wrap(buf).order(ByteOrder.LITTLE_ENDIAN)
            val w = bb.int
            val h = bb.int
            val gen = bb.long
            if (w <= 0 || h <= 0) return null
            return Header(w, h, gen)
        }
    }

    private fun readRgba(path: String, width: Int, height: Int): ByteArray? {
        val expected = SHM_HEADER_SIZE + width * height * 4
        val file = File(path)
        if (file.length() < expected) return null
        val out = ByteArray(width * height * 4)
        RandomAccessFile(file, "r").use { raf ->
            raf.seek(SHM_HEADER_SIZE.toLong())
            if (raf.read(out) != out.size) return null
        }
        return out
    }
}
