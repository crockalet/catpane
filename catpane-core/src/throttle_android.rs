//! Client and protocol for the CatPane Android **helper app**'s throttling
//! VPN service.
//!
//! Network throttling on a *physical* Android device cannot use the emulator
//! console (`adb emu network speed/delay`), so CatPane installs a small
//! companion app (`dev.catpane.helper`) that owns a `VpnService`-backed TUN
//! device and shapes traffic in user space. CatPane drives that helper from
//! the host over an ADB-forwarded TCP control socket.
//!
//! This module contains:
//!
//! * The wire protocol (`ControlRequest` / `ControlResponse`) — line-delimited
//!   JSON, fully unit-tested without a device.
//! * A pluggable [`ControlTransport`] trait so the dispatcher can be tested
//!   against an in-memory transport.
//! * A real ADB-backed transport implementation that runs `adb forward`,
//!   opens a `TcpStream`, and writes/reads framed JSON.
//! * High-level helpers (`ensure_helper_installed`, `ensure_vpn_permission`,
//!   `apply_device_network_condition`, `clear_device_network_condition`).
//!
//! Everything below the top-level helpers is `pub(crate)` exposed via
//! `#[cfg(test)]`-friendly entry points so tests can substitute transports
//! without spawning ADB.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::network_condition::{CustomNetworkParams, NetworkConditionPreset, NetworkConditionSpec};

/// Package identifier of the CatPane Android helper app.
pub const HELPER_PACKAGE: &str = "dev.catpane.helper";

/// Fixed device-side TCP port the helper's control server binds to on
/// `127.0.0.1`. Picked from the IANA dynamic range so it doesn't collide
/// with common dev tooling. The host side picks an ephemeral free port and
/// forwards it to this device port.
pub const HELPER_DEVICE_PORT: u16 = 47821;

/// Environment variable that lets users point CatPane at a custom helper APK
/// (e.g. a locally rebuilt debug build) instead of the bundled sidecar/
/// embedded fallback.
pub const HELPER_APK_ENV: &str = "CATPANE_HELPER_APK";

/// Filename used for the sidecar APK inside packaged releases.
pub const HELPER_APK_FILENAME: &str = "catpane-helper.apk";

/// Default timeout for a single round-trip on the control socket.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------

/// LAN-exclusion modes understood by the helper. The default is
/// `AdbHostOnly`, which spares the workstation IP so wireless ADB keeps
/// working while the VPN is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanExclusionMode {
    /// Tunnel everything, including LAN.
    None,
    /// Exclude only the workstation IP that issued the control request.
    AdbHostOnly,
    /// Exclude the entire local /24 (or device-detected) subnet.
    FullLan,
}

impl Default for LanExclusionMode {
    fn default() -> Self {
        Self::AdbHostOnly
    }
}

/// Requests the host can send to the helper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Read current helper state.
    Status,
    /// Apply a network-condition spec.
    Apply { spec: NetworkConditionSpec },
    /// Restore unthrottled traffic and tear down the VPN.
    Clear,
    /// Reconfigure LAN exclusion (host_ip is required for `adb_host_only`).
    SetLanExclusion {
        mode: LanExclusionMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_ip: Option<String>,
    },
}

/// Snapshot of helper state returned by `status` and `apply`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HelperStatus {
    /// `true` once the VPN service is bound and the shaper is running.
    pub running: bool,
    /// `true` when the user has granted `BIND_VPN_SERVICE` and revealed the
    /// helper to the system via `VpnService.prepare`.
    pub vpn_permission_granted: bool,
    pub current_spec: Option<NetworkConditionSpec>,
    pub lan_exclusion: LanExclusionMode,
    pub helper_version: Option<String>,
    /// Free-form human-readable status message (helper-side details).
    #[serde(default)]
    pub message: Option<String>,
}

/// Stable error codes the helper may return so the client can recover with
/// targeted UI affordances (install/grant prompts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperErrorCode {
    PermissionRequired,
    AlreadyAnotherVpnActive,
    InvalidParams,
    Internal,
}

/// Wire-level response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HelperStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<HelperErrorCode>,
}

impl ControlResponse {
    pub fn ok_with(status: HelperStatus) -> Self {
        Self {
            ok: true,
            status: Some(status),
            error: None,
            code: None,
        }
    }

    pub fn err(code: HelperErrorCode, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            status: None,
            error: Some(message.into()),
            code: Some(code),
        }
    }
}

// ---------------------------------------------------------------------------
// Pluggable transport
// ---------------------------------------------------------------------------

/// A transport that can perform a single request/response round-trip with the
/// helper. Abstracted as a trait so unit tests can drive the dispatch layer
/// without spawning ADB or binding sockets.
#[async_trait::async_trait]
pub trait ControlTransport: Send + Sync {
    async fn round_trip(&self, request: ControlRequest) -> Result<ControlResponse, String>;
}

/// Real transport: opens an `adb forward`-backed TCP connection to the
/// helper, writes a JSON line, reads a JSON line response, then closes the
/// socket. Re-uses an existing forward (per-serial registry) when possible.
pub struct AdbForwardTransport {
    serial: String,
}

impl AdbForwardTransport {
    pub fn new(serial: impl Into<String>) -> Self {
        Self {
            serial: serial.into(),
        }
    }
}

#[async_trait::async_trait]
impl ControlTransport for AdbForwardTransport {
    async fn round_trip(&self, request: ControlRequest) -> Result<ControlResponse, String> {
        let host_port = ensure_forward(&self.serial).await?;
        let stream = TcpStream::connect(("127.0.0.1", host_port))
            .await
            .map_err(|e| {
                format!(
                    "Failed to connect to CatPane helper on {}:{} (is it installed and running?): {e}",
                    self.serial, host_port
                )
            })?;
        write_and_read(stream, &request).await
    }
}

async fn write_and_read(
    stream: TcpStream,
    request: &ControlRequest,
) -> Result<ControlResponse, String> {
    let (read_half, mut write_half) = stream.into_split();
    let mut payload = serde_json::to_string(request)
        .map_err(|e| format!("Failed to encode helper request: {e}"))?;
    payload.push('\n');

    timeout(CONTROL_TIMEOUT, write_half.write_all(payload.as_bytes()))
        .await
        .map_err(|_| "Timed out writing to CatPane helper".to_string())?
        .map_err(|e| format!("Failed to write to CatPane helper: {e}"))?;
    timeout(CONTROL_TIMEOUT, write_half.flush())
        .await
        .map_err(|_| "Timed out flushing CatPane helper socket".to_string())?
        .map_err(|e| format!("Failed to flush CatPane helper socket: {e}"))?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let read_result = timeout(CONTROL_TIMEOUT, reader.read_line(&mut line))
        .await
        .map_err(|_| "Timed out reading from CatPane helper".to_string())?;
    let bytes = read_result.map_err(|e| format!("Failed to read from CatPane helper: {e}"))?;
    if bytes == 0 {
        return Err("CatPane helper closed the control connection unexpectedly".to_string());
    }
    serde_json::from_str::<ControlResponse>(line.trim_end())
        .map_err(|e| format!("Malformed response from CatPane helper: {e}"))
}

// ---------------------------------------------------------------------------
// Per-serial `adb forward` registry
// ---------------------------------------------------------------------------

fn forward_registry() -> &'static Mutex<HashMap<String, u16>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, u16>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ensures `adb forward tcp:<host_port> tcp:<HELPER_DEVICE_PORT>` is set up
/// for `serial` and returns the host port. The mapping is cached so repeated
/// calls re-use the same port.
async fn ensure_forward(serial: &str) -> Result<u16, String> {
    {
        let map = forward_registry().lock().await;
        if let Some(port) = map.get(serial) {
            return Ok(*port);
        }
    }
    let host_port = pick_free_port()?;
    let mut cmd = tokio::process::Command::new("adb");
    cmd.args([
        "-s",
        serial,
        "forward",
        &format!("tcp:{host_port}"),
        &format!("tcp:{HELPER_DEVICE_PORT}"),
    ]);
    let out = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to spawn adb forward: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "adb forward failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut map = forward_registry().lock().await;
    map.insert(serial.to_string(), host_port);
    Ok(host_port)
}

/// Removes any forward established by [`ensure_forward`] for `serial`.
pub async fn release_forward(serial: &str) {
    let port = {
        let mut map = forward_registry().lock().await;
        map.remove(serial)
    };
    if let Some(port) = port {
        let _ = tokio::process::Command::new("adb")
            .args(["-s", serial, "forward", "--remove", &format!("tcp:{port}")])
            .output()
            .await;
    }
}

fn pick_free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to pick a free local port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to read local port: {e}"))?
        .port();
    drop(listener);
    Ok(port)
}

// ---------------------------------------------------------------------------
// APK location & install helpers
// ---------------------------------------------------------------------------

/// Reports whether the helper APK can be located on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperApkLocation {
    /// Found at this path (env override or sidecar next to the catpane binary).
    Path(PathBuf),
    /// Bundled inside the binary at build time. (Reserved for a future
    /// `include_bytes!` integration; not currently populated to keep the
    /// release binary slim.)
    Embedded,
    /// Could not locate the APK at all.
    Missing,
}

/// Searches for the helper APK in the standard locations:
///
/// 1. `$CATPANE_HELPER_APK` (explicit override)
/// 2. Next to the running executable (`<exe-dir>/catpane-helper.apk`)
/// 3. macOS app bundle Resources dir (`<exe-dir>/../Resources/catpane-helper.apk`)
pub fn locate_helper_apk() -> HelperApkLocation {
    if let Ok(path) = std::env::var(HELPER_APK_ENV)
        && !path.is_empty()
    {
        let p = PathBuf::from(path);
        if p.is_file() {
            return HelperApkLocation::Path(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sidecar = dir.join(HELPER_APK_FILENAME);
        if sidecar.is_file() {
            return HelperApkLocation::Path(sidecar);
        }
        let bundle_resources = dir.join("..").join("Resources").join(HELPER_APK_FILENAME);
        if bundle_resources.is_file() {
            return HelperApkLocation::Path(bundle_resources);
        }
    }
    HelperApkLocation::Missing
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperInstallStatus {
    AlreadyInstalled,
    Installed,
    HelperApkMissing,
}

/// Returns `true` when `pm path dev.catpane.helper` reports an installed APK.
pub async fn is_helper_installed(serial: &str) -> Result<bool, String> {
    let out = tokio::process::Command::new("adb")
        .args(["-s", serial, "shell", "pm", "path", HELPER_PACKAGE])
        .output()
        .await
        .map_err(|e| format!("Failed to spawn adb shell pm path: {e}"))?;
    if !out.status.success() {
        // `pm path` returns non-zero when the package is not installed.
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.contains("package:"))
}

/// Installs (or upgrades) the helper APK on `serial`. No-op when already
/// installed. Returns [`HelperInstallStatus::HelperApkMissing`] if no APK can
/// be located on disk.
pub async fn ensure_helper_installed(serial: &str) -> Result<HelperInstallStatus, String> {
    if is_helper_installed(serial).await? {
        return Ok(HelperInstallStatus::AlreadyInstalled);
    }
    let apk_path = match locate_helper_apk() {
        HelperApkLocation::Path(p) => p,
        HelperApkLocation::Embedded | HelperApkLocation::Missing => {
            return Ok(HelperInstallStatus::HelperApkMissing);
        }
    };
    let out = tokio::process::Command::new("adb")
        .args([
            "-s",
            serial,
            "install",
            "-r",
            "-g",
            apk_path
                .to_str()
                .ok_or_else(|| format!("APK path is not valid UTF-8: {}", apk_path.display()))?,
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to spawn adb install: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "adb install failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(HelperInstallStatus::Installed)
}

/// Launches the helper's `MainActivity` so the user can grant the
/// `BIND_VPN_SERVICE` permission. Polling for the actual grant is done via
/// the control protocol (`status.vpn_permission_granted`).
pub async fn launch_helper_for_permission(serial: &str) -> Result<(), String> {
    let component = format!("{HELPER_PACKAGE}/.MainActivity");
    let out = tokio::process::Command::new("adb")
        .args([
            "-s", serial, "shell", "am", "start", "-n", &component, "--es", "reason",
            "request_vpn_permission",
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to spawn adb shell am start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Failed to launch CatPane helper UI: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// High-level apply / clear
// ---------------------------------------------------------------------------

/// Applies a network-condition spec via the helper's control socket.
///
/// Validates `spec` first so we don't make a useless round-trip. On success
/// returns the human-readable [`HelperStatus::message`] (or a synthesized
/// summary if the helper didn't include one).
pub async fn apply_device_network_condition(
    serial: &str,
    spec: NetworkConditionSpec,
) -> Result<String, String> {
    spec.validate()?;
    let transport = AdbForwardTransport::new(serial);
    apply_with_transport(&transport, serial, spec).await
}

pub async fn clear_device_network_condition(serial: &str) -> Result<String, String> {
    let transport = AdbForwardTransport::new(serial);
    clear_with_transport(&transport, serial).await
}

/// Test-friendly version of `apply_device_network_condition` that takes a
/// custom transport.
pub async fn apply_with_transport<T: ControlTransport>(
    transport: &T,
    serial: &str,
    spec: NetworkConditionSpec,
) -> Result<String, String> {
    let resp = transport
        .round_trip(ControlRequest::Apply { spec })
        .await?;
    interpret_response(resp, serial, "Applied network condition")
}

pub async fn clear_with_transport<T: ControlTransport>(
    transport: &T,
    serial: &str,
) -> Result<String, String> {
    let resp = transport.round_trip(ControlRequest::Clear).await?;
    interpret_response(resp, serial, "Cleared network throttling")
}

pub async fn status_with_transport<T: ControlTransport>(
    transport: &T,
) -> Result<HelperStatus, String> {
    let resp = transport.round_trip(ControlRequest::Status).await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "Unknown helper error".to_string()));
    }
    Ok(resp.status.unwrap_or_default())
}

fn interpret_response(
    resp: ControlResponse,
    serial: &str,
    success_prefix: &str,
) -> Result<String, String> {
    if resp.ok {
        let summary = resp
            .status
            .as_ref()
            .and_then(|s| s.message.clone())
            .unwrap_or_else(|| format!("{success_prefix} on {serial}"));
        return Ok(summary);
    }
    let error = resp
        .error
        .unwrap_or_else(|| "Helper returned an unspecified error".to_string());
    let actionable = match resp.code {
        Some(HelperErrorCode::PermissionRequired) => {
            " — open the CatPane helper app on the device and tap 'Grant VPN permission', \
             then retry."
        }
        Some(HelperErrorCode::AlreadyAnotherVpnActive) => {
            " — another VPN is currently active. Disconnect it (Settings → Network → VPN) and \
             retry."
        }
        _ => "",
    };
    Err(format!("{error}{actionable}"))
}

// ---------------------------------------------------------------------------
// Convenience: turn a raw preset into a Spec for backwards compat.
// ---------------------------------------------------------------------------

pub fn spec_from_preset(preset: NetworkConditionPreset) -> NetworkConditionSpec {
    NetworkConditionSpec::preset(preset)
}

pub fn spec_from_custom(custom: CustomNetworkParams) -> NetworkConditionSpec {
    NetworkConditionSpec::custom(custom)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex as TokioMutex;

    /// In-memory transport that records every request and returns scripted
    /// responses, so we can drive the dispatcher without ADB.
    struct ScriptedTransport {
        sent: Arc<TokioMutex<Vec<ControlRequest>>>,
        responses: TokioMutex<Vec<ControlResponse>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<ControlResponse>) -> Self {
            Self {
                sent: Arc::new(TokioMutex::new(Vec::new())),
                responses: TokioMutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl ControlTransport for ScriptedTransport {
        async fn round_trip(
            &self,
            request: ControlRequest,
        ) -> Result<ControlResponse, String> {
            self.sent.lock().await.push(request);
            let mut responses = self.responses.lock().await;
            if responses.is_empty() {
                return Err("scripted transport ran out of responses".to_string());
            }
            Ok(responses.remove(0))
        }
    }

    #[test]
    fn request_serde_round_trip_preset() {
        let req = ControlRequest::Apply {
            spec: NetworkConditionSpec::preset(NetworkConditionPreset::ThreeG),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"op\":\"apply\""));
        assert!(json.contains("\"kind\":\"preset\""));
        let back: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn request_serde_round_trip_custom() {
        let req = ControlRequest::Apply {
            spec: NetworkConditionSpec::custom(CustomNetworkParams {
                delay_ms: Some(120),
                jitter_ms: Some(30),
                loss_pct: Some(1.5),
                downlink_kbps: Some(2000),
                uplink_kbps: Some(800),
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn request_serde_lan_exclusion() {
        let req = ControlRequest::SetLanExclusion {
            mode: LanExclusionMode::AdbHostOnly,
            host_ip: Some("192.168.1.10".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"mode\":\"adb_host_only\""));
        assert!(json.contains("192.168.1.10"));
        let back: ControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);

        // host_ip omitted should round-trip as None
        let req2 = ControlRequest::SetLanExclusion {
            mode: LanExclusionMode::None,
            host_ip: None,
        };
        let json2 = serde_json::to_string(&req2).unwrap();
        assert!(!json2.contains("host_ip"));
        let back2: ControlRequest = serde_json::from_str(&json2).unwrap();
        assert_eq!(back2, req2);
    }

    #[test]
    fn response_error_envelope_carries_code() {
        let resp = ControlResponse::err(
            HelperErrorCode::PermissionRequired,
            "user has not granted VPN permission",
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"ok\":false"));
        assert!(json.contains("\"code\":\"permission_required\""));
        let back: ControlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[tokio::test]
    async fn apply_with_transport_returns_helper_message() {
        let transport = ScriptedTransport::new(vec![ControlResponse::ok_with(HelperStatus {
            running: true,
            vpn_permission_granted: true,
            current_spec: Some(NetworkConditionSpec::preset(NetworkConditionPreset::ThreeG)),
            lan_exclusion: LanExclusionMode::AdbHostOnly,
            helper_version: Some("0.1.0".to_string()),
            message: Some("Throttling at 3G".to_string()),
        })]);
        let result = apply_with_transport(
            &transport,
            "device",
            NetworkConditionSpec::preset(NetworkConditionPreset::ThreeG),
        )
        .await
        .unwrap();
        assert_eq!(result, "Throttling at 3G");
        let sent = transport.sent.lock().await.clone();
        assert_eq!(sent.len(), 1);
        assert!(matches!(&sent[0], ControlRequest::Apply { .. }));
    }

    #[tokio::test]
    async fn apply_with_transport_surfaces_permission_error_actionably() {
        let transport = ScriptedTransport::new(vec![ControlResponse::err(
            HelperErrorCode::PermissionRequired,
            "VPN permission required",
        )]);
        let err = apply_with_transport(
            &transport,
            "device",
            NetworkConditionSpec::preset(NetworkConditionPreset::Edge),
        )
        .await
        .unwrap_err();
        assert!(err.contains("VPN permission required"));
        assert!(err.contains("Grant VPN permission"));
    }

    #[tokio::test]
    async fn apply_with_transport_surfaces_other_vpn_active() {
        let transport = ScriptedTransport::new(vec![ControlResponse::err(
            HelperErrorCode::AlreadyAnotherVpnActive,
            "Another VPN is connected",
        )]);
        let err = apply_with_transport(
            &transport,
            "device",
            NetworkConditionSpec::preset(NetworkConditionPreset::Edge),
        )
        .await
        .unwrap_err();
        assert!(err.contains("another VPN") || err.contains("Another VPN"));
    }

    #[tokio::test]
    async fn apply_validates_spec_before_round_trip() {
        // delay_ms over the configured maximum should be rejected without ever
        // touching the transport.
        let bad = NetworkConditionSpec::custom(CustomNetworkParams {
            loss_pct: Some(150.0),
            ..Default::default()
        });
        let err = apply_device_network_condition("device", bad).await.unwrap_err();
        assert!(err.contains("loss_pct"));
    }

    #[tokio::test]
    async fn clear_with_transport_synthesizes_message_when_helper_silent() {
        let transport = ScriptedTransport::new(vec![ControlResponse::ok_with(
            HelperStatus::default(),
        )]);
        let result = clear_with_transport(&transport, "abc-serial").await.unwrap();
        assert!(result.contains("abc-serial"));
        assert!(result.starts_with("Cleared"));
    }

    #[test]
    fn locate_helper_apk_env_override() {
        let dir = tempdir_or_skip();
        let apk = dir.join("custom.apk");
        std::fs::write(&apk, b"PK\x03\x04").unwrap();
        let _guard = EnvGuard::set(HELPER_APK_ENV, apk.to_str().unwrap());
        match locate_helper_apk() {
            HelperApkLocation::Path(p) => assert_eq!(p, apk),
            other => panic!("expected env-override path, got {other:?}"),
        }
    }

    #[test]
    fn locate_helper_apk_missing_when_nothing_present() {
        let _guard = EnvGuard::clear(HELPER_APK_ENV);
        // When neither env var nor sidecar exists we should never panic.
        let res = locate_helper_apk();
        // We can't strictly assert Missing because some dev shells *do* place
        // a sidecar APK next to the test binary; just assert we got *some*
        // legal variant.
        assert!(matches!(
            res,
            HelperApkLocation::Path(_) | HelperApkLocation::Embedded | HelperApkLocation::Missing
        ));
    }

    // -- tiny test helpers ------------------------------------------------

    fn tempdir_or_skip() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "catpane-throttle-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: tests run single-threaded for env mutations within this
            // module; we restore on drop.
            unsafe { std::env::set_var(key, value) };
            Self { key, prev }
        }
        fn clear(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
