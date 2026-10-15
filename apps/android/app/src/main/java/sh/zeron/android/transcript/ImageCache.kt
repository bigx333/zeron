package sh.zeron.android.transcript

import android.content.ComponentCallbacks2
import android.util.LruCache
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asAndroidBitmap

/**
 * Decoded attachment images, shared by thumbnails and the full-size viewer.
 * Bounded by decoded bytes (a full-resolution screenshot is ~10 MB), and keyed
 * by host device and path so equal paths on two hosts never share an entry.
 */
object ImageCache {
    private val cache = object : LruCache<String, ImageBitmap>(
        (Runtime.getRuntime().maxMemory() / 8 / 1024).coerceIn(1, Int.MAX_VALUE.toLong()).toInt(),
    ) {
        // Round up in KB, including row padding and higher-bit-depth images.
        override fun sizeOf(key: String, value: ImageBitmap) =
            ((value.asAndroidBitmap().allocationByteCount.toLong() + 1023) / 1024).toInt().coerceAtLeast(1)
    }

    fun key(device: String, reference: String) = "$device\u0000$reference"

    operator fun get(key: String): ImageBitmap? = cache.get(key)

    fun put(key: String, image: ImageBitmap) {
        cache.put(key, image)
    }

    /** Shed images under foreground pressure as well as when leaving the UI. */
    fun trim(level: Int) {
        when {
            level >= ComponentCallbacks2.TRIM_MEMORY_BACKGROUND ||
                level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL -> cache.evictAll()
            level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW -> cache.trimToSize(cache.maxSize() / 2)
        }
    }
}
