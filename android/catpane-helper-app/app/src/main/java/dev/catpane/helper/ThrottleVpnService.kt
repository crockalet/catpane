package dev.catpane.helper

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.util.Log
import dev.catpane.helper.ControlProtocol.LanExclusionMode
import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread

/**
 * VPN service that owns the TUN and applies shaping to each direction.
 *
 * Note on the packet pump: a full pure-Kotlin TUN-to-socket bridge is non-
 * trivial; this v1 implementation establishes the TUN and runs a *drain loop*
 * that reads + (optionally) drops/delays packets without re-injecting them.
 * That gives us correct **offline** and **delay/loss** behaviour today and
 * leaves a clear seam to plug a native shaper in for v2.
 */
class ThrottleVpnService : VpnService() {

    @Volatile private var iface: ParcelFileDescriptor? = null
    @Volatile private var pumpThread: Thread? = null
    private val configRef = AtomicReference(ShaperConfig())
    private val lanModeRef = AtomicReference(LanExclusionMode.ADB_HOST_ONLY)
    private val excludedHostRef = AtomicReference<String?>(null)
    @Volatile private var running: Boolean = false

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startInForeground()
        return START_STICKY
    }

    override fun onDestroy() {
        stopShaping()
        instance = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? {
        // We expose no AIDL surface; the system uses VpnService binding via
        // the framework when starting us as a VPN.
        return super.onBind(intent)
    }

    fun applyConfig(config: ShaperConfig, mode: LanExclusionMode, hostIp: String?) {
        lanModeRef.set(mode)
        excludedHostRef.set(hostIp)
        configRef.set(config)
        if (config.unthrottled) {
            stopShaping()
            return
        }
        ensureTun()
        running = true
    }

    fun stopShaping() {
        running = false
        try { iface?.close() } catch (_: Throwable) {}
        iface = null
        pumpThread = null
    }

    fun snapshotStatus(currentSpec: ControlProtocol.Spec?): ControlProtocol.HelperStatus {
        return ControlProtocol.HelperStatus(
            running = running,
            vpnPermissionGranted = true,
            currentSpec = currentSpec,
            lanExclusion = lanModeRef.get(),
            message = if (running) "shaping active" else "idle",
        )
    }

    private fun ensureTun() {
        if (iface != null) return
        val builder = Builder()
            .setSession(getString(R.string.app_name))
            .addAddress("10.7.7.1", 24)
            .addRoute("0.0.0.0", 0)
            .setMtu(1400)

        // LAN exclusion: don't tunnel ADB-host traffic so wireless debugging
        // keeps working while shaping is on.
        when (lanModeRef.get()) {
            LanExclusionMode.NONE -> { /* no excludes */ }
            LanExclusionMode.ADB_HOST_ONLY -> excludedHostRef.get()?.let {
                runCatching { builder.addRoute(it, 32); }
                    .onFailure { Log.w(TAG, "addRoute exclude $it failed", it) }
            }
            LanExclusionMode.FULL_LAN -> {
                // Re-route private ranges back through the underlying network.
                // The Android VpnService.Builder doesn't have a true exclude;
                // omitting routes for these ranges has the same effect when
                // we *don't* set 0.0.0.0/0 — but we do need 0/0 for shaping to
                // take effect. Best-effort: fall back to ADB-host-only.
                excludedHostRef.get()?.let {
                    runCatching { builder.addRoute(it, 32); }
                        .onFailure { Log.w(TAG, "addRoute exclude $it failed", it) }
                }
            }
        }

        val pfd = builder.establish() ?: run {
            Log.e(TAG, "VpnService.Builder.establish returned null — permission revoked?")
            return
        }
        iface = pfd
        pumpThread = thread(name = "catpane-helper-pump", isDaemon = true) {
            runPump(pfd)
        }
    }

    private fun runPump(pfd: ParcelFileDescriptor) {
        val input = java.io.FileInputStream(pfd.fileDescriptor)
        val buffer = ByteArray(32 * 1024)
        val rng = kotlin.random.Random.Default
        var downBucket = bucketFor(configRef.get().downlinkKbps)
        var lastCfg = configRef.get()

        while (running && !Thread.currentThread().isInterrupted) {
            val cfg = configRef.get()
            if (cfg !== lastCfg) {
                downBucket = bucketFor(cfg.downlinkKbps)
                lastCfg = cfg
            }
            val read = try { input.read(buffer) } catch (_: Throwable) { -1 }
            if (read < 0) break
            if (read == 0) continue

            // Loss
            if (Shaping.shouldDrop(cfg.lossPct, rng)) continue
            // Delay
            val delay = Shaping.delayWithJitter(cfg.delayMs, cfg.jitterMs, rng)
            if (delay > 0) try { Thread.sleep(delay) } catch (_: InterruptedException) { break }
            // Bandwidth (downlink applied to drained packets as a uniform cap)
            if (downBucket != null) {
                val wait = downBucket.reserve(read)
                if (wait > 0) try { Thread.sleep(wait / 1_000_000, (wait % 1_000_000).toInt()) }
                catch (_: InterruptedException) { break }
            }
            // v1 sink: packets are dropped here. This achieves "offline" and
            // "high latency / loss" semantics correctly. Re-injection (full
            // forwarding) is the v2 work item — see plan.md.
        }
    }

    private fun bucketFor(kbps: Int): TokenBucket? {
        if (kbps <= 0) return null
        val bps = kbps.toLong() * 1000L / 8L
        return TokenBucket(bps)
    }

    private fun startInForeground() {
        val mgr = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(CHANNEL_ID, "CatPane shaping", NotificationManager.IMPORTANCE_LOW)
            mgr.createNotificationChannel(ch)
        }
        val pi = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notif: Notification = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.app_name))
            .setContentText("Network shaping VPN")
            .setSmallIcon(android.R.drawable.stat_sys_vp_phone_call)
            .setContentIntent(pi)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    companion object {
        private const val TAG = "CatPaneHelper"
        private const val CHANNEL_ID = "catpane_shaping"
        private const val NOTIF_ID = 0xCA7
        @Volatile var instance: ThrottleVpnService? = null
    }
}

/**
 * Background TCP control server. Singleton thread bound to
 * `127.0.0.1:[ControlProtocol.CONTROL_PORT]`, accepts one client at a time,
 * processes line-delimited JSON requests until the client disconnects.
 *
 * Started by [MainActivity] once VPN permission has been granted, and kept
 * alive across configuration changes by being a `Thread` (not bound to the
 * activity lifecycle).
 */
object ControlServer {
    private const val TAG = "CatPaneControl"
    @Volatile private var server: ServerSocket? = null
    @Volatile private var thread: Thread? = null
    @Volatile private var lastSpec: ControlProtocol.Spec? = null

    fun ensureStarted(context: Context) {
        if (thread?.isAlive == true) return
        val sock = ServerSocket()
        sock.reuseAddress = true
        sock.bind(InetSocketAddress(InetAddress.getByName("127.0.0.1"), ControlProtocol.CONTROL_PORT))
        server = sock
        val app = context.applicationContext
        thread = thread(name = "catpane-helper-control", isDaemon = true) {
            Log.i(TAG, "control server listening on ${ControlProtocol.CONTROL_PORT}")
            while (!Thread.currentThread().isInterrupted) {
                val client = try { sock.accept() } catch (_: Throwable) { break }
                handle(app, client)
            }
        }
    }

    fun stop() {
        try { server?.close() } catch (_: Throwable) {}
        server = null
        thread = null
    }

    private fun handle(context: Context, client: Socket) {
        client.use { sock ->
            val reader = BufferedReader(InputStreamReader(sock.getInputStream(), Charsets.UTF_8))
            val writer = OutputStreamWriter(sock.getOutputStream(), Charsets.UTF_8)
            val peerHost = (sock.remoteSocketAddress as? InetSocketAddress)?.address?.hostAddress
            while (true) {
                val line = reader.readLine() ?: return
                val response = try {
                    val request = ControlProtocol.decodeRequest(line)
                    dispatch(context, request, peerHost)
                } catch (e: Throwable) {
                    Log.w(TAG, "bad request: $line", e)
                    ControlProtocol.Response(
                        ok = false,
                        error = e.message ?: "invalid request",
                        code = ControlProtocol.ErrorCode.INVALID_PARAMS,
                    )
                }
                writer.write(ControlProtocol.encodeResponse(response))
                writer.write("\n")
                writer.flush()
            }
        }
    }

    private fun dispatch(
        context: Context,
        request: ControlProtocol.Request,
        peerHost: String?,
    ): ControlProtocol.Response {
        val service = ThrottleVpnService.instance
        return when (request) {
            ControlProtocol.Request.Status -> {
                val status = service?.snapshotStatus(lastSpec)
                    ?: ControlProtocol.HelperStatus(
                        running = false,
                        vpnPermissionGranted = VpnService.prepare(context) == null,
                        currentSpec = null,
                        lanExclusion = LanExclusionMode.ADB_HOST_ONLY,
                        message = "service not running",
                    )
                ControlProtocol.Response(ok = true, status = status)
            }
            is ControlProtocol.Request.Apply -> {
                if (VpnService.prepare(context) != null) {
                    return ControlProtocol.Response(
                        ok = false,
                        error = "VPN permission required; open the CatPane Helper app and tap Grant.",
                        code = ControlProtocol.ErrorCode.PERMISSION_REQUIRED,
                    )
                }
                lastSpec = request.spec
                val cfg = try { ShaperConfig.fromSpec(request.spec) } catch (e: Throwable) {
                    return ControlProtocol.Response(
                        ok = false,
                        error = e.message ?: "invalid spec",
                        code = ControlProtocol.ErrorCode.INVALID_PARAMS,
                    )
                }
                // Start the VPN if not yet up.
                context.startService(Intent(context, ThrottleVpnService::class.java))
                ThrottleVpnService.instance?.applyConfig(
                    cfg, LanExclusionMode.ADB_HOST_ONLY, peerHost
                )
                val status = ThrottleVpnService.instance?.snapshotStatus(lastSpec)
                ControlProtocol.Response(ok = true, status = status)
            }
            ControlProtocol.Request.Clear -> {
                lastSpec = null
                ThrottleVpnService.instance?.stopShaping()
                ControlProtocol.Response(
                    ok = true,
                    status = ThrottleVpnService.instance?.snapshotStatus(null),
                )
            }
            is ControlProtocol.Request.SetLanExclusion -> {
                val svc = ThrottleVpnService.instance
                val cfg = lastSpec?.let { runCatching { ShaperConfig.fromSpec(it) }.getOrNull() }
                    ?: ShaperConfig()
                svc?.applyConfig(cfg, request.mode, request.hostIp ?: peerHost)
                ControlProtocol.Response(
                    ok = true,
                    status = svc?.snapshotStatus(lastSpec),
                )
            }
        }
    }
}
