package dev.catpane.helper

import kotlin.math.max
import kotlin.random.Random

/**
 * Pure-Kotlin shaper config + token-bucket. The [VpnService] packet pump uses
 * this to apply per-direction bandwidth caps, randomized delay/jitter and
 * packet-loss probability.
 *
 * Kept dependency-free so it can be unit-tested on the JVM without an
 * Android device.
 */
data class ShaperConfig(
    val delayMs: Int = 0,
    val jitterMs: Int = 0,
    val lossPct: Double = 0.0,
    val downlinkKbps: Int = 0,
    val uplinkKbps: Int = 0,
) {
    val unthrottled: Boolean
        get() = delayMs == 0 && jitterMs == 0 && lossPct == 0.0 &&
            downlinkKbps == 0 && uplinkKbps == 0

    companion object {
        fun fromSpec(spec: ControlProtocol.Spec): ShaperConfig = when (spec) {
            is ControlProtocol.Spec.Preset -> presetConfig(spec.preset)
            is ControlProtocol.Spec.Custom -> ShaperConfig(
                delayMs = spec.delayMs ?: 0,
                jitterMs = spec.jitterMs ?: 0,
                lossPct = spec.lossPct ?: 0.0,
                downlinkKbps = spec.downlinkKbps ?: 0,
                uplinkKbps = spec.uplinkKbps ?: 0,
            )
        }

        // Mirrors the parity values used by the iOS Simulator presets so users
        // observe roughly the same symptoms across device kinds.
        private fun presetConfig(slug: String): ShaperConfig = when (slug.lowercase()) {
            "unthrottled", "none", "full" -> ShaperConfig()
            "edge" -> ShaperConfig(
                delayMs = 400, jitterMs = 100, lossPct = 0.5,
                downlinkKbps = 240, uplinkKbps = 200
            )
            "3g" -> ShaperConfig(
                delayMs = 200, jitterMs = 50, lossPct = 0.2,
                downlinkKbps = 1500, uplinkKbps = 750
            )
            "offline" -> ShaperConfig(
                delayMs = 0, jitterMs = 0, lossPct = 100.0,
                downlinkKbps = 0, uplinkKbps = 0
            )
            else -> throw IllegalArgumentException("unknown preset: $slug")
        }
    }
}

/**
 * Token-bucket bandwidth limiter. Capacity in bytes; refill rate in bytes/sec.
 * Returns the number of nanoseconds the caller must wait before being allowed
 * to send `bytes`. Returns 0 when no throttling is active.
 */
class TokenBucket(
    private val rateBytesPerSec: Long,
    capacityBytes: Long = max(1L, rateBytesPerSec / 4),
    private val nowNanos: () -> Long = { System.nanoTime() },
) {
    private val capacity: Long = if (rateBytesPerSec <= 0) Long.MAX_VALUE else capacityBytes
    private var tokens: Double = capacity.toDouble()
    private var lastRefill: Long = nowNanos()

    @Synchronized
    fun reserve(bytes: Int): Long {
        if (rateBytesPerSec <= 0L) return 0L
        refill()
        if (tokens >= bytes) {
            tokens -= bytes
            return 0L
        }
        val deficit = bytes - tokens
        // seconds = deficit / rate ; nanos = deficit / rate * 1e9
        val waitNanos = (deficit / rateBytesPerSec.toDouble() * 1_000_000_000.0).toLong()
        tokens = 0.0
        return waitNanos
    }

    private fun refill() {
        val now = nowNanos()
        val elapsedNs = now - lastRefill
        if (elapsedNs <= 0) return
        val add = (elapsedNs / 1_000_000_000.0) * rateBytesPerSec
        tokens = (tokens + add).coerceAtMost(capacity.toDouble())
        lastRefill = now
    }
}

/** Random-jitter / loss helpers extracted for unit testing. */
object Shaping {
    fun shouldDrop(lossPct: Double, rng: Random = Random.Default): Boolean {
        if (lossPct <= 0.0) return false
        if (lossPct >= 100.0) return true
        return rng.nextDouble() * 100.0 < lossPct
    }

    /** Total delay (ms) = base ± jitter (uniform). Never negative. */
    fun delayWithJitter(delayMs: Int, jitterMs: Int, rng: Random = Random.Default): Long {
        if (delayMs <= 0 && jitterMs <= 0) return 0L
        val j = if (jitterMs <= 0) 0 else rng.nextInt(-jitterMs, jitterMs + 1)
        return max(0, delayMs + j).toLong()
    }
}
