use std::net::UdpSocket;
use tokio::process::Command;
use tokio::sync::mpsc;

/// Resolves the `adb` binary path, probing common macOS installation locations
/// so that GUI apps (Homebrew Cask, double-click launch) find adb even when
/// the shell PATH is not inherited.
pub fn adb_binary() -> &'static str {
    static ADB_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ADB_PATH.get_or_init(|| {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(android_sdk_root) = std::env::var("ANDROID_SDK_ROOT") {
            candidates.push(format!("{android_sdk_root}/platform-tools/adb").into());
        }
        if let Ok(android_home) = std::env::var("ANDROID_HOME") {
            candidates.push(format!("{android_home}/platform-tools/adb").into());
        }
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(format!("{home}/Library/Android/Sdk/platform-tools/adb").into());
            candidates.push(format!("{home}/Library/Android/sdk/platform-tools/adb").into());
        }
        candidates.push("/opt/homebrew/bin/adb".into());
        candidates.push("/usr/local/bin/adb".into());

        for path in &candidates {
            if path.exists() {
                return path.to_string_lossy().into_owned();
            }
        }

        "adb".to_string()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrPairEvent {
    Status(String),
    Finished(Result<String, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpipEnableResult {
    pub message: String,
    pub connect_host: Option<String>,
    pub connected: bool,
}

impl std::fmt::Display for AdbDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.friendly_name())
    }
}

impl AdbDevice {
    /// Extract a human-readable name from the adb description.
    /// Parses fields like "model:Pixel_6" from `adb devices -l` output.
    pub fn friendly_name(&self) -> String {
        if self.description.is_empty() {
            return self.serial.clone();
        }

        for part in self.description.split_whitespace() {
            if let Some(val) = part.strip_prefix("model:") {
                return val.replace('_', " ");
            }
        }

        for part in self.description.split_whitespace() {
            if let Some(val) = part.strip_prefix("device:") {
                return val.to_string();
            }
        }

        self.serial.clone()
    }
}

/// Returns true if `serial` is a TCP/IP connection (IP:port format).
/// USB serials and mDNS serials (`adb-XXX._adb-tls-connect._tcp`) do not contain `:`.
pub fn is_tcp_device(serial: &str) -> bool {
    serial.contains(':')
}

/// Deduplicate devices that refer to the same physical hardware.
/// When wireless debugging is active, `adb devices -l` emits both an IP:port entry
/// and an mDNS service-name entry for the same device. Keep only the IP:port one.
fn deduplicate_devices(devices: Vec<AdbDevice>) -> Vec<AdbDevice> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut result: Vec<AdbDevice> = Vec::new();

    for device in devices {
        let name = device.friendly_name();
        if let Some(&idx) = seen.get(&name) {
            // Prefer IP:port serial over mDNS/USB serial
            if is_tcp_device(&device.serial) && !is_tcp_device(&result[idx].serial) {
                result[idx] = device;
            }
        } else {
            seen.insert(name, result.len());
            result.push(device);
        }
    }

    result
}

pub async fn list_devices() -> Vec<AdbDevice> {
    let output = match Command::new(adb_binary())
        .args(["devices", "-l"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let raw: Vec<AdbDevice> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // Find " device " as the status separator (with spaces to avoid matching "device:" in descriptions)
            // Line format: <serial> [mDNS info] device <description key:value pairs>
            let idx = line.find(" device ")?;
            let serial = line[..idx].trim().to_string();
            let description = line[idx + 8..].trim().to_string();
            Some(AdbDevice {
                serial,
                description,
            })
        })
        .collect();

    deduplicate_devices(raw)
}

pub async fn list_packages(device: &str) -> Vec<String> {
    // Try running processes first
    if let Ok(output) = Command::new(adb_binary())
        .args(["-s", device, "shell", "ps", "-A", "-o", "NAME"])
        .output()
        .await
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut packages: Vec<String> = stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let name = line.trim();
                if name.contains('.') && !name.starts_with('[') {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect();
        packages.sort();
        packages.dedup();
        if !packages.is_empty() {
            return packages;
        }
    }

    // Fallback: all installed packages
    let output = match Command::new(adb_binary())
        .args(["-s", device, "shell", "pm", "list", "packages"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let mut pkgs: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("package:").map(|p| p.trim().to_string()))
        .collect();
    pkgs.sort();
    pkgs
}

pub async fn get_pid_for_package(device: &str, package: &str) -> Option<u32> {
    let output = Command::new(adb_binary())
        .args(["-s", device, "shell", "pidof", package])
        .output()
        .await
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Pair with a device using `adb pair host:port code`.
pub async fn pair_device(host_port: &str, code: &str) -> Result<String, String> {
    let output = Command::new(adb_binary())
        .args(["pair", host_port, code])
        .output()
        .await
        .map_err(|e| format!("Failed to run adb pair: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr).to_lowercase();

    if combined.contains("success") {
        Ok(stdout.trim().to_string())
    } else {
        Err(format!("{}{}", stdout.trim(), stderr.trim()))
    }
}

/// Connect to a device using `adb connect host:port`.
pub async fn connect_device(host_port: &str) -> Result<String, String> {
    let output = Command::new(adb_binary())
        .args(["connect", host_port])
        .output()
        .await
        .map_err(|e| format!("Failed to run adb connect: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() && stdout.to_lowercase().contains("connected") {
        Ok(stdout.trim().to_string())
    } else {
        Err(format!("{}{}", stdout.trim(), stderr.trim()))
    }
}

/// Disconnect a wireless device using `adb disconnect host:port`.
pub async fn disconnect_device(serial: &str) -> Result<String, String> {
    let output = Command::new(adb_binary())
        .args(["disconnect", serial])
        .output()
        .await
        .map_err(|e| format!("Failed to run adb disconnect: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if stdout.to_lowercase().contains("disconnected") || output.status.success() {
        Ok(stdout.trim().to_string())
    } else {
        Err(format!("{}{}", stdout.trim(), stderr.trim()))
    }
}

/// Restart the adb server.
pub async fn restart_server() -> Result<String, String> {
    Command::new(adb_binary())
        .args(["kill-server"])
        .output()
        .await
        .map_err(|e| format!("Failed to run adb kill-server: {e}"))?;

    let output = Command::new(adb_binary())
        .args(["start-server"])
        .output()
        .await
        .map_err(|e| format!("Failed to run adb start-server: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        let msg = if stdout.is_empty() {
            "ADB server restarted".to_string()
        } else {
            stdout
        };
        Ok(msg)
    } else {
        Err(format!("{stdout}{stderr}"))
    }
}

/// Enable adb TCP/IP mode over USB and attempt to auto-connect to the device's Wi-Fi IP.
pub async fn enable_tcpip_mode(device: &str, port: u16) -> Result<TcpipEnableResult, String> {
    let output = Command::new(adb_binary())
        .args(["-s", device, "tcpip", &port.to_string()])
        .output()
        .await
        .map_err(|e| format!("Failed to run adb tcpip: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!("{stdout}{stderr}"));
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let ip = detect_device_ip(device).await.ok();
    let connect_host = ip.map(|ip| format!("{ip}:{port}"));

    if let Some(host) = &connect_host {
        match connect_device(host).await {
            Ok(_) => {
                return Ok(TcpipEnableResult {
                    message: format!("Enabled TCP/IP on {device} and connected to {host}."),
                    connect_host,
                    connected: true,
                });
            }
            Err(err) => {
                return Ok(TcpipEnableResult {
                    message: format!(
                        "Enabled TCP/IP on {device}, but automatic connect to {host} failed: {err}"
                    ),
                    connect_host,
                    connected: false,
                });
            }
        }
    }

    let prefix = if stdout.is_empty() {
        format!("Enabled TCP/IP on {device}.")
    } else {
        stdout
    };
    Ok(TcpipEnableResult {
        message: format!(
            "{prefix} Connect manually using the device Wi-Fi IP and port {port}."
        ),
        connect_host: None,
        connected: false,
    })
}

/// Generate a random alphabetic string (letters only, like lyto).
pub fn random_id(len: usize) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let chars: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut result = String::with_capacity(len);
    let mut state = seed;
    for _ in 0..len {
        // Simple xorshift for randomness
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        result.push(chars[(state as usize) % chars.len()] as char);
    }
    result
}

/// Generate the ADB pairing QR code string.
pub fn qr_pairing_string(service_name: &str, password: &str) -> String {
    format!("WIFI:T:ADB;S:{service_name};P:{password};;")
}

/// Generate a QR code as an egui-compatible ColorImage.
pub fn generate_qr_image(data: &str, scale: usize) -> egui::ColorImage {
    use qrcode::QrCode;

    let code = QrCode::new(data.as_bytes()).unwrap();
    let modules = code.to_colors();
    let width = code.width();
    let img_size = width * scale + scale * 2; // add 1-module quiet zone on each side

    let mut pixels = vec![egui::Color32::WHITE; img_size * img_size];

    for y in 0..width {
        for x in 0..width {
            let color = if modules[y * width + x] == qrcode::Color::Dark {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            };
            // Draw scaled pixel with quiet zone offset
            for dy in 0..scale {
                for dx in 0..scale {
                    let px = (x + 1) * scale + dx; // +1 for quiet zone
                    let py = (y + 1) * scale + dy;
                    pixels[py * img_size + px] = color;
                }
            }
        }
    }

    egui::ColorImage {
        size: [img_size, img_size],
        pixels,
    }
}

/// Start mDNS discovery for ADB pairing services.
///
/// Follows the lyto/adb-wifi approach:
/// 1. Browse both `_adb-tls-connect._tcp` and `_adb-tls-pairing._tcp`
/// 2. Collect the connect port from the connect service first
/// 3. When the pairing service appears, run `adb pair` with our password
/// 4. Then auto-connect using the connect port
pub fn spawn_mdns_pairing_discovery(
    _rt: &tokio::runtime::Handle,
    expected_name: String,
    password: String,
) -> mpsc::Receiver<QrPairEvent> {
    let (tx, rx) = mpsc::channel::<QrPairEvent>(8);
    let adb_path = adb_binary().to_string();

    std::thread::spawn(move || {
        use mdns_sd::{ServiceDaemon, ServiceEvent};
        use std::collections::HashSet;

        let mdns = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.blocking_send(QrPairEvent::Finished(Err(format!(
                    "mDNS init failed: {e}"
                ))));
                return;
            }
        };

        let pairing_type = "_adb-tls-pairing._tcp.local.";
        let connect_type = "_adb-tls-connect._tcp.local.";

        let pair_rx = match mdns.browse(pairing_type) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.blocking_send(QrPairEvent::Finished(Err(format!(
                    "mDNS browse (pair) failed: {e}"
                ))));
                return;
            }
        };
        let conn_rx = match mdns.browse(connect_type) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.blocking_send(QrPairEvent::Finished(Err(format!(
                    "mDNS browse (connect) failed: {e}"
                ))));
                return;
            }
        };

        let _ = tx.blocking_send(QrPairEvent::Status(
            "Waiting for the phone to advertise the QR pairing service…".to_string(),
        ));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut connect_ports: Vec<(String, u16)> = Vec::new(); // (addr, port)
        let mut tried: HashSet<String> = HashSet::new();

        loop {
            if std::time::Instant::now() > deadline {
                let _ = tx.blocking_send(QrPairEvent::Finished(Err(
                    "Timed out waiting for the QR pairing service".to_string(),
                )));
                break;
            }

            // Poll connect service (non-blocking)
            while let Ok(event) = conn_rx.try_recv() {
                if let ServiceEvent::ServiceResolved(info) = event {
                    if let Some(addr) = info.get_addresses().iter().find(|a| a.is_ipv4()) {
                        let port = info.get_port();
                        let addr = addr.to_string();
                        eprintln!("mDNS: found connect service at {}:{}", addr, port);
                        if !connect_ports.iter().any(|entry| entry == &(addr.clone(), port)) {
                            connect_ports.push((addr, port));
                        }
                    }
                }
            }

            // Poll pairing service
            match pair_rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if !service_matches_expected_name(&info.get_fullname(), &expected_name) {
                        continue;
                    }

                    // Need at least one connect port before pairing (like lyto)
                    if connect_ports.is_empty() {
                        eprintln!(
                            "mDNS: pairing service found but no connect port yet, waiting..."
                        );
                        // Still try to pair — some setups may work without connect port
                    }

                    let port = info.get_port();
                    let addr = match info.get_addresses().iter().find(|a| a.is_ipv4()) {
                        Some(a) => a.to_string(),
                        None => continue,
                    };

                    let pair_addr = format!("{}:{}", addr, port);
                    if !tried.insert(pair_addr.clone()) {
                        continue;
                    }

                    eprintln!("mDNS: attempting adb pair {}...", pair_addr);
                    let _ = tx.blocking_send(QrPairEvent::Status(format!(
                        "Found the QR pairing service at {pair_addr}; attempting to pair…"
                    )));

                    let output = match std::process::Command::new(&adb_path)
                        .args(["pair", &pair_addr, &password])
                        .output()
                    {
                        Ok(o) => o,
                        Err(e) => {
                            eprintln!("adb pair command error: {e}");
                            let _ = tx.blocking_send(QrPairEvent::Finished(Err(format!(
                                "Failed to run adb pair: {e}"
                            ))));
                            continue;
                        }
                    };

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let combined = format!("{}{}", stdout, stderr);

                    if combined.to_lowercase().contains("success") {
                        // Pair succeeded — now auto-connect
                        if let Some((conn_addr, conn_port)) = connect_ports
                            .iter()
                            .rev()
                            .find(|(conn_addr, _)| conn_addr == &addr)
                        {
                            let connect_addr = format!("{}:{}", conn_addr, conn_port);
                            eprintln!("Paired! Auto-connecting to {}...", connect_addr);
                            let _ = tx.blocking_send(QrPairEvent::Status(format!(
                                "Paired successfully; connecting to {connect_addr}…"
                            )));
                            let connect_output = std::process::Command::new(&adb_path)
                                .args(["connect", &connect_addr])
                                .output();
                            match connect_output {
                                Ok(output) => {
                                    let connect_stdout = String::from_utf8_lossy(&output.stdout);
                                    let connect_stderr = String::from_utf8_lossy(&output.stderr);
                                    let connect_combined =
                                        format!("{}{}", connect_stdout, connect_stderr);
                                    if output.status.success()
                                        && connect_combined.to_lowercase().contains("connected")
                                    {
                                        let _ = tx.blocking_send(QrPairEvent::Finished(Ok(
                                            format!("Paired & connected to {connect_addr}"),
                                        )));
                                    } else {
                                        let _ = tx.blocking_send(QrPairEvent::Finished(Ok(
                                            format!(
                                                "Paired with {pair_addr}. Automatic connect to \
                                                 {connect_addr} failed: {}. Use the Connect \
                                                 section with the device's connect port if needed.",
                                                connect_combined.trim()
                                            ),
                                        )));
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.blocking_send(QrPairEvent::Finished(Ok(format!(
                                        "Paired with {pair_addr}. Automatic connect to \
                                         {connect_addr} failed: {e}. Use the Connect section if \
                                         needed."
                                    ))));
                                }
                            }
                        } else {
                            let _ = tx.blocking_send(QrPairEvent::Finished(Ok(format!(
                                "Paired with {pair_addr}. If the device does not appear \
                                 automatically, use the Connect section with the device's \
                                 connect port."
                            ))));
                        }
                        break;
                    } else {
                        eprintln!(
                            "Pair attempt with {} failed: {}",
                            pair_addr,
                            combined.trim()
                        );
                        let _ = tx.blocking_send(QrPairEvent::Finished(Err(combined.trim().to_string())));
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }

        let _ = mdns.shutdown();
    });

    rx
}

/// Detect the local IP prefix (e.g. "192.168.1.") by briefly opening a UDP socket.
/// Returns empty string if detection fails.
pub fn local_ip_prefix() -> String {
    // Connect to a public DNS — no data is sent, just determines the local route
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    if socket.connect("8.8.8.8:80").is_err() {
        return String::new();
    }
    match socket.local_addr() {
        Ok(addr) => {
            let ip = addr.ip().to_string();
            match ip.rfind('.') {
                Some(pos) => ip[..=pos].to_string(),
                None => String::new(),
            }
        }
        Err(_) => String::new(),
    }
}

fn service_matches_expected_name(fullname: &str, expected_name: &str) -> bool {
    fullname.contains(expected_name)
}

async fn detect_device_ip(device: &str) -> Result<String, String> {
    let output = Command::new(adb_binary())
        .args(["-s", device, "shell", "ip", "route"])
        .output()
        .await
        .map_err(|e| format!("Failed to query device IP: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    extract_ip_from_route_output(&stdout).ok_or_else(|| {
        "Could not determine the device Wi-Fi IP from `adb shell ip route`".to_string()
    })
}

fn extract_ip_from_route_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        while let Some(part) = parts.next() {
            if part == "src" {
                let ip = parts.next()?;
                if ip.parse::<std::net::IpAddr>().is_ok() {
                    return Some(ip.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{extract_ip_from_route_output, service_matches_expected_name};

    #[test]
    fn extracts_ip_from_route_output() {
        let output = "192.168.0.0/24 dev wlan0 proto kernel scope link src 192.168.0.15";
        assert_eq!(
            extract_ip_from_route_output(output).as_deref(),
            Some("192.168.0.15")
        );
    }

    #[test]
    fn matches_expected_mdns_service_name() {
        assert!(service_matches_expected_name(
            "ADB_WIFI_abcde._adb-tls-pairing._tcp.local.",
            "ADB_WIFI_abcde"
        ));
        assert!(!service_matches_expected_name(
            "Pixel_8._adb-tls-pairing._tcp.local.",
            "ADB_WIFI_abcde"
        ));
    }
}
