package dev.catpane.helper

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import kotlin.random.Random

class ControlProtocolTest {

    @Test
    fun decodes_status_request() {
        val req = ControlProtocol.decodeRequest("""{"op":"status"}""")
        assertTrue(req is ControlProtocol.Request.Status)
    }

    @Test
    fun decodes_clear_request() {
        val req = ControlProtocol.decodeRequest("""{"op":"clear"}""")
        assertTrue(req is ControlProtocol.Request.Clear)
    }

    @Test
    fun decodes_apply_preset() {
        val req = ControlProtocol.decodeRequest(
            """{"op":"apply","spec":{"kind":"preset","preset":"3g"}}"""
        )
        require(req is ControlProtocol.Request.Apply)
        require(req.spec is ControlProtocol.Spec.Preset)
        assertEquals("3g", (req.spec as ControlProtocol.Spec.Preset).preset)
    }

    @Test
    fun decodes_apply_custom_with_all_params() {
        val req = ControlProtocol.decodeRequest(
            """{"op":"apply","spec":{"kind":"custom","delay_ms":120,"jitter_ms":30,"loss_pct":1.5,"downlink_kbps":2000,"uplink_kbps":800}}"""
        )
        require(req is ControlProtocol.Request.Apply)
        val custom = req.spec as ControlProtocol.Spec.Custom
        assertEquals(120, custom.delayMs)
        assertEquals(30, custom.jitterMs)
        assertEquals(1.5, custom.lossPct!!, 0.0001)
        assertEquals(2000, custom.downlinkKbps)
        assertEquals(800, custom.uplinkKbps)
    }

    @Test
    fun decodes_apply_custom_partial_fields() {
        val req = ControlProtocol.decodeRequest(
            """{"op":"apply","spec":{"kind":"custom","delay_ms":250}}"""
        )
        require(req is ControlProtocol.Request.Apply)
        val custom = req.spec as ControlProtocol.Spec.Custom
        assertEquals(250, custom.delayMs)
        assertNull(custom.jitterMs)
        assertNull(custom.lossPct)
    }

    @Test
    fun decodes_set_lan_exclusion_default_mode() {
        // Missing host_ip is fine; missing mode defaults to AdbHostOnly per Rust default.
        val req = ControlProtocol.decodeRequest("""{"op":"set_lan_exclusion","mode":"adb_host_only"}""")
        require(req is ControlProtocol.Request.SetLanExclusion)
        assertEquals(ControlProtocol.LanExclusionMode.ADB_HOST_ONLY, req.mode)
        assertNull(req.hostIp)
    }

    @Test
    fun decodes_set_lan_exclusion_full_lan_with_host() {
        val req = ControlProtocol.decodeRequest(
            """{"op":"set_lan_exclusion","mode":"full_lan","host_ip":"192.168.1.42"}"""
        )
        require(req is ControlProtocol.Request.SetLanExclusion)
        assertEquals(ControlProtocol.LanExclusionMode.FULL_LAN, req.mode)
        assertEquals("192.168.1.42", req.hostIp)
    }

    @Test
    fun rejects_unknown_op() {
        try {
            ControlProtocol.decodeRequest("""{"op":"nope"}""")
            fail("expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {}
    }

    @Test
    fun rejects_apply_without_spec() {
        try {
            ControlProtocol.decodeRequest("""{"op":"apply"}""")
            fail("expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {}
    }

    @Test
    fun encodes_ok_response_with_status() {
        val resp = ControlProtocol.Response(
            ok = true,
            status = ControlProtocol.HelperStatus(
                running = true,
                vpnPermissionGranted = true,
                currentSpec = ControlProtocol.Spec.Preset("3g"),
                lanExclusion = ControlProtocol.LanExclusionMode.ADB_HOST_ONLY,
            ),
        )
        val json = ControlProtocol.encodeResponse(resp)
        assertTrue(json.contains("\"ok\":true"))
        assertTrue(json.contains("\"running\":true"))
        assertTrue(json.contains("\"vpn_permission_granted\":true"))
        assertTrue(json.contains("\"kind\":\"preset\""))
        assertTrue(json.contains("\"preset\":\"3g\""))
        assertTrue(json.contains("\"lan_exclusion\":\"adb_host_only\""))
    }

    @Test
    fun encodes_error_response_with_code() {
        val resp = ControlProtocol.Response(
            ok = false,
            error = "no permission",
            code = ControlProtocol.ErrorCode.PERMISSION_REQUIRED,
        )
        val json = ControlProtocol.encodeResponse(resp)
        assertTrue(json.contains("\"ok\":false"))
        assertTrue(json.contains("\"error\":\"no permission\""))
        assertTrue(json.contains("\"code\":\"permission_required\""))
    }

    @Test
    fun encodes_custom_spec_omits_null_fields() {
        val json = ControlProtocol.encodeSpec(
            ControlProtocol.Spec.Custom(delayMs = 100)
        )
        assertEquals("""{"kind":"custom","delay_ms":100}""", json)
    }

    @Test
    fun string_escaping_handles_quotes_and_newlines() {
        val resp = ControlProtocol.Response(ok = false, error = "bad \"thing\"\nnext")
        val json = ControlProtocol.encodeResponse(resp)
        assertTrue(json.contains("""\"thing\""""))
        assertTrue(json.contains("""\n"""))
    }
}

class ShaperConfigTest {

    @Test
    fun preset_unthrottled_yields_zero_config() {
        val cfg = ShaperConfig.fromSpec(ControlProtocol.Spec.Preset("unthrottled"))
        assertTrue(cfg.unthrottled)
    }

    @Test
    fun preset_3g_has_delay_and_bandwidth() {
        val cfg = ShaperConfig.fromSpec(ControlProtocol.Spec.Preset("3g"))
        assertFalse(cfg.unthrottled)
        assertTrue(cfg.delayMs > 0)
        assertTrue(cfg.downlinkKbps > 0)
    }

    @Test
    fun preset_offline_drops_everything() {
        val cfg = ShaperConfig.fromSpec(ControlProtocol.Spec.Preset("offline"))
        assertEquals(100.0, cfg.lossPct, 0.0)
    }

    @Test
    fun custom_spec_passes_through_set_fields() {
        val cfg = ShaperConfig.fromSpec(
            ControlProtocol.Spec.Custom(delayMs = 250, lossPct = 5.0)
        )
        assertEquals(250, cfg.delayMs)
        assertEquals(5.0, cfg.lossPct, 0.0)
        assertEquals(0, cfg.uplinkKbps)
    }

    @Test
    fun preset_alias_full_maps_to_unthrottled() {
        assertTrue(ShaperConfig.fromSpec(ControlProtocol.Spec.Preset("full")).unthrottled)
        assertTrue(ShaperConfig.fromSpec(ControlProtocol.Spec.Preset("none")).unthrottled)
    }

    @Test
    fun unknown_preset_throws() {
        try {
            ShaperConfig.fromSpec(ControlProtocol.Spec.Preset("satellite"))
            fail("expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {}
    }
}

class ShapingTest {

    @Test
    fun loss_zero_never_drops() {
        val rng = Random(42)
        repeat(1000) {
            assertFalse(Shaping.shouldDrop(0.0, rng))
        }
    }

    @Test
    fun loss_hundred_always_drops() {
        val rng = Random(42)
        repeat(1000) {
            assertTrue(Shaping.shouldDrop(100.0, rng))
        }
    }

    @Test
    fun loss_50_pct_is_roughly_half() {
        val rng = Random(7)
        var drops = 0
        repeat(10_000) { if (Shaping.shouldDrop(50.0, rng)) drops++ }
        assertTrue("expected ~5000 drops, got $drops", drops in 4500..5500)
    }

    @Test
    fun delay_with_no_jitter_returns_base() {
        assertEquals(150L, Shaping.delayWithJitter(150, 0))
        assertEquals(0L, Shaping.delayWithJitter(0, 0))
    }

    @Test
    fun delay_with_jitter_within_bounds() {
        val rng = Random(0)
        repeat(500) {
            val v = Shaping.delayWithJitter(100, 30, rng)
            assertTrue("$v out of range", v in 70L..130L)
        }
    }

    @Test
    fun delay_never_negative() {
        val rng = Random(0)
        repeat(500) {
            val v = Shaping.delayWithJitter(5, 100, rng)
            assertTrue("$v < 0", v >= 0)
        }
    }
}

class TokenBucketTest {

    @Test
    fun zero_rate_means_unlimited() {
        val tb = TokenBucket(rateBytesPerSec = 0)
        assertEquals(0L, tb.reserve(1_000_000))
    }

    @Test
    fun small_request_within_capacity_is_immediate() {
        val tb = TokenBucket(rateBytesPerSec = 1000, capacityBytes = 1000) { 0L }
        assertEquals(0L, tb.reserve(500))
    }

    @Test
    fun overspend_returns_positive_wait() {
        // rate = 1000 B/s, capacity = 1000 B, fake clock pinned at 0
        var now = 0L
        val tb = TokenBucket(rateBytesPerSec = 1000, capacityBytes = 1000) { now }
        // First call drains the bucket.
        assertEquals(0L, tb.reserve(1000))
        // Second call with no time elapsed should owe ~1s = 1e9 ns.
        val wait = tb.reserve(1000)
        assertTrue("wait was $wait", wait >= 900_000_000L && wait <= 1_100_000_000L)
    }

    @Test
    fun refill_after_time_passes_reduces_wait() {
        var now = 0L
        val tb = TokenBucket(rateBytesPerSec = 1000, capacityBytes = 1000) { now }
        tb.reserve(1000)              // drain
        now = 500_000_000L            // 0.5s later → 500 B refilled
        val wait = tb.reserve(1000)   // need 500 more B → ~0.5s wait
        assertTrue("wait was $wait", wait in 400_000_000L..600_000_000L)
    }
}
