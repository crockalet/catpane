package dev.catpane.helper

/**
 * Wire protocol shared with CatPane (`catpane-core/src/throttle_android.rs`).
 *
 * The protocol is line-delimited JSON over a TCP socket bound to
 * `127.0.0.1:[CONTROL_PORT]`. The host reaches it via `adb forward`.
 *
 * We hand-roll the JSON encoder/decoder to avoid dragging kotlinx.serialization
 * into the helper APK — keeping it small matters because it ships embedded in
 * CatPane releases.
 */
object ControlProtocol {

    const val CONTROL_PORT: Int = 47821
    const val HELPER_VERSION: String = "0.1.0"

    enum class LanExclusionMode(val wire: String) {
        NONE("none"),
        ADB_HOST_ONLY("adb_host_only"),
        FULL_LAN("full_lan");

        companion object {
            fun fromWire(value: String?): LanExclusionMode = when (value) {
                "none" -> NONE
                "adb_host_only", null -> ADB_HOST_ONLY
                "full_lan" -> FULL_LAN
                else -> throw IllegalArgumentException("unknown lan_exclusion mode: $value")
            }
        }
    }

    enum class ErrorCode(val wire: String) {
        PERMISSION_REQUIRED("permission_required"),
        ALREADY_ANOTHER_VPN_ACTIVE("already_another_vpn_active"),
        INVALID_PARAMS("invalid_params"),
        INTERNAL("internal"),
    }

    /** Spec is either {kind:"preset", preset:"3g"} or {kind:"custom", delay_ms:..., ...}. */
    sealed interface Spec {
        data class Preset(val preset: String) : Spec
        data class Custom(
            val delayMs: Int? = null,
            val jitterMs: Int? = null,
            val lossPct: Double? = null,
            val downlinkKbps: Int? = null,
            val uplinkKbps: Int? = null,
        ) : Spec {
            fun isUnthrottled(): Boolean =
                delayMs == null && jitterMs == null && lossPct == null &&
                    downlinkKbps == null && uplinkKbps == null
        }
    }

    sealed interface Request {
        object Status : Request
        data class Apply(val spec: Spec) : Request
        object Clear : Request
        data class SetLanExclusion(val mode: LanExclusionMode, val hostIp: String?) : Request
    }

    data class HelperStatus(
        val running: Boolean,
        val vpnPermissionGranted: Boolean,
        val currentSpec: Spec?,
        val lanExclusion: LanExclusionMode,
        val helperVersion: String? = HELPER_VERSION,
        val message: String? = null,
    )

    data class Response(
        val ok: Boolean,
        val status: HelperStatus? = null,
        val error: String? = null,
        val code: ErrorCode? = null,
    )

    // -----------------------------------------------------------------------
    // Encoding
    // -----------------------------------------------------------------------

    fun encodeResponse(response: Response): String {
        val sb = StringBuilder("{\"ok\":${response.ok}")
        response.status?.let { sb.append(",\"status\":${encodeStatus(it)}") }
        response.error?.let { sb.append(",\"error\":${jsonString(it)}") }
        response.code?.let { sb.append(",\"code\":${jsonString(it.wire)}") }
        sb.append('}')
        return sb.toString()
    }

    fun encodeStatus(status: HelperStatus): String {
        val sb = StringBuilder("{")
        sb.append("\"running\":${status.running}")
        sb.append(",\"vpn_permission_granted\":${status.vpnPermissionGranted}")
        sb.append(",\"current_spec\":")
        if (status.currentSpec == null) sb.append("null") else sb.append(encodeSpec(status.currentSpec))
        sb.append(",\"lan_exclusion\":${jsonString(status.lanExclusion.wire)}")
        status.helperVersion?.let { sb.append(",\"helper_version\":${jsonString(it)}") }
        status.message?.let { sb.append(",\"message\":${jsonString(it)}") }
        sb.append('}')
        return sb.toString()
    }

    fun encodeSpec(spec: Spec): String {
        return when (spec) {
            is Spec.Preset -> "{\"kind\":\"preset\",\"preset\":${jsonString(spec.preset)}}"
            is Spec.Custom -> {
                val sb = StringBuilder("{\"kind\":\"custom\"")
                spec.delayMs?.let { sb.append(",\"delay_ms\":$it") }
                spec.jitterMs?.let { sb.append(",\"jitter_ms\":$it") }
                spec.lossPct?.let { sb.append(",\"loss_pct\":$it") }
                spec.downlinkKbps?.let { sb.append(",\"downlink_kbps\":$it") }
                spec.uplinkKbps?.let { sb.append(",\"uplink_kbps\":$it") }
                sb.append('}')
                sb.toString()
            }
        }
    }

    // -----------------------------------------------------------------------
    // Decoding
    // -----------------------------------------------------------------------

    fun decodeRequest(line: String): Request {
        val obj = JsonParser(line).parseObject()
        val op = obj["op"] as? String ?: throw IllegalArgumentException("missing 'op'")
        return when (op) {
            "status" -> Request.Status
            "clear" -> Request.Clear
            "apply" -> {
                @Suppress("UNCHECKED_CAST")
                val spec = obj["spec"] as? Map<String, Any?>
                    ?: throw IllegalArgumentException("apply requires 'spec'")
                Request.Apply(decodeSpec(spec))
            }
            "set_lan_exclusion" -> {
                val mode = LanExclusionMode.fromWire(obj["mode"] as? String)
                val hostIp = obj["host_ip"] as? String
                Request.SetLanExclusion(mode, hostIp)
            }
            else -> throw IllegalArgumentException("unknown op: $op")
        }
    }

    fun decodeSpec(map: Map<String, Any?>): Spec {
        return when (val kind = map["kind"] as? String) {
            "preset" -> Spec.Preset(
                map["preset"] as? String
                    ?: throw IllegalArgumentException("preset spec missing 'preset'")
            )
            "custom" -> Spec.Custom(
                delayMs = (map["delay_ms"] as? Number)?.toInt(),
                jitterMs = (map["jitter_ms"] as? Number)?.toInt(),
                lossPct = (map["loss_pct"] as? Number)?.toDouble(),
                downlinkKbps = (map["downlink_kbps"] as? Number)?.toInt(),
                uplinkKbps = (map["uplink_kbps"] as? Number)?.toInt(),
            )
            else -> throw IllegalArgumentException("unknown spec kind: $kind")
        }
    }

    private fun jsonString(value: String): String {
        val sb = StringBuilder("\"")
        for (c in value) {
            when (c) {
                '\\' -> sb.append("\\\\")
                '"' -> sb.append("\\\"")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                '\b' -> sb.append("\\b")
                else -> if (c.code < 0x20) {
                    sb.append("\\u%04x".format(c.code))
                } else {
                    sb.append(c)
                }
            }
        }
        sb.append('"')
        return sb.toString()
    }
}

/** Tiny hand-rolled JSON parser sufficient for our line-delimited protocol. */
internal class JsonParser(private val src: String) {
    private var pos: Int = 0

    fun parseObject(): Map<String, Any?> {
        skipWs()
        expect('{')
        val out = LinkedHashMap<String, Any?>()
        skipWs()
        if (peek() == '}') {
            pos++
            return out
        }
        while (true) {
            skipWs()
            val key = parseString()
            skipWs()
            expect(':')
            val value = parseValue()
            out[key] = value
            skipWs()
            when (val c = next()) {
                ',' -> continue
                '}' -> return out
                else -> throw IllegalArgumentException("expected ',' or '}' got '$c' at $pos")
            }
        }
    }

    private fun parseValue(): Any? {
        skipWs()
        return when (val c = peek()) {
            '"' -> parseString()
            '{' -> parseObject()
            '[' -> parseArray()
            't', 'f' -> parseBoolean()
            'n' -> { expectKeyword("null"); null }
            else -> if (c == '-' || c.isDigit()) parseNumber() else
                throw IllegalArgumentException("unexpected '$c' at $pos")
        }
    }

    private fun parseArray(): List<Any?> {
        expect('[')
        val out = mutableListOf<Any?>()
        skipWs()
        if (peek() == ']') { pos++; return out }
        while (true) {
            out += parseValue()
            skipWs()
            when (val c = next()) {
                ',' -> continue
                ']' -> return out
                else -> throw IllegalArgumentException("expected ',' or ']' got '$c'")
            }
        }
    }

    private fun parseString(): String {
        expect('"')
        val sb = StringBuilder()
        while (true) {
            val c = next()
            if (c == '"') return sb.toString()
            if (c == '\\') {
                when (val esc = next()) {
                    '"' -> sb.append('"')
                    '\\' -> sb.append('\\')
                    '/' -> sb.append('/')
                    'n' -> sb.append('\n')
                    'r' -> sb.append('\r')
                    't' -> sb.append('\t')
                    'b' -> sb.append('\b')
                    'u' -> {
                        val hex = src.substring(pos, pos + 4); pos += 4
                        sb.append(hex.toInt(16).toChar())
                    }
                    else -> throw IllegalArgumentException("bad escape '\\$esc'")
                }
            } else {
                sb.append(c)
            }
        }
    }

    private fun parseNumber(): Number {
        val start = pos
        if (peek() == '-') pos++
        while (pos < src.length && src[pos].isDigit()) pos++
        var isFloat = false
        if (pos < src.length && src[pos] == '.') {
            isFloat = true; pos++
            while (pos < src.length && src[pos].isDigit()) pos++
        }
        if (pos < src.length && (src[pos] == 'e' || src[pos] == 'E')) {
            isFloat = true; pos++
            if (pos < src.length && (src[pos] == '+' || src[pos] == '-')) pos++
            while (pos < src.length && src[pos].isDigit()) pos++
        }
        val text = src.substring(start, pos)
        return if (isFloat) text.toDouble() else text.toLong()
    }

    private fun parseBoolean(): Boolean {
        return if (peek() == 't') { expectKeyword("true"); true } else { expectKeyword("false"); false }
    }

    private fun expectKeyword(word: String) {
        if (pos + word.length > src.length || src.substring(pos, pos + word.length) != word)
            throw IllegalArgumentException("expected '$word' at $pos")
        pos += word.length
    }

    private fun expect(c: Char) {
        val got = next()
        if (got != c) throw IllegalArgumentException("expected '$c' got '$got' at $pos")
    }

    private fun next(): Char {
        if (pos >= src.length) throw IllegalArgumentException("unexpected EOF")
        return src[pos++]
    }

    private fun peek(): Char {
        if (pos >= src.length) throw IllegalArgumentException("unexpected EOF")
        return src[pos]
    }

    private fun skipWs() {
        while (pos < src.length && src[pos].isWhitespace()) pos++
    }
}
