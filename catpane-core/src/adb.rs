use std::{net::UdpSocket, process::Output, time::Duration};
use tokio::sync::mpsc;

use crate::command::OneShotCommand;
use crate::network_condition::NetworkConditionPreset;

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

const ADB_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const ADB_DEVICE_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const ADB_CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);
const ADB_NETWORK_TIMEOUT: Duration = Duration::from_secs(10);

fn adb_command<I, S>(args: I, context: &'static str, timeout: Duration) -> OneShotCommand
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    OneShotCommand::new(adb_binary(), args, context, timeout)
}

fn combine_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
        (true, true) => String::new(),
    }
}

fn parse_adb_devices_output(stdout: &[u8]) -> Vec<AdbDevice> {
    let raw: Vec<AdbDevice> = String::from_utf8_lossy(stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
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

fn parse_running_packages_output(stdout: &[u8]) -> Vec<String> {
    let mut packages: Vec<String> = String::from_utf8_lossy(stdout)
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
    packages
}

fn parse_installed_packages_output(stdout: &[u8]) -> Vec<String> {
    let mut packages: Vec<String> = String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("package:").map(|p| p.trim().to_string()))
        .collect();
    packages.sort();
    packages
}

fn parse_pidof_output(stdout: &[u8]) -> Option<u32> {
    String::from_utf8_lossy(stdout)
        .trim()
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub async fn list_devices_strict() -> Result<Vec<AdbDevice>, String> {
    let command = adb_command(
        ["devices", "-l"],
        "listing connected Android devices",
        ADB_DISCOVERY_TIMEOUT,
    );
    let output = command.ensure_success(command.run().await?)?;
    Ok(parse_adb_devices_output(&output.stdout))
}

pub async fn list_devices() -> Vec<AdbDevice> {
    list_devices_strict().await.unwrap_or_default()
}

pub async fn list_packages_strict(device: &str) -> Result<Vec<String>, String> {
    let ps_command = adb_command(
        ["-s", device, "shell", "ps", "-A", "-o", "NAME"],
        "listing running Android packages",
        ADB_DEVICE_QUERY_TIMEOUT,
    );
    let ps_error = match ps_command.run().await {
        Ok(output) => match ps_command.ensure_success(output) {
            Ok(output) => {
                let packages = parse_running_packages_output(&output.stdout);
                if !packages.is_empty() {
                    return Ok(packages);
                }
                None
            }
            Err(err) => Some(err),
        },
        Err(err) => Some(err),
    };

    let pm_command = adb_command(
        ["-s", device, "shell", "pm", "list", "packages"],
        "listing installed Android packages",
        ADB_DEVICE_QUERY_TIMEOUT,
    );
    let output = match pm_command.run().await {
        Ok(output) => match pm_command.ensure_success(output) {
            Ok(output) => output,
            Err(err) => {
                return Err(match ps_error {
                    Some(ps_error) => {
                        format!("{err}; running-process lookup failed earlier with: {ps_error}")
                    }
                    None => err,
                });
            }
        },
        Err(err) => {
            return Err(match ps_error {
                Some(ps_error) => {
                    format!("{err}; fallback from running-process lookup failed after: {ps_error}")
                }
                None => err,
            });
        }
    };
    let packages = parse_installed_packages_output(&output.stdout);

    if packages.is_empty() {
        if let Some(ps_error) = ps_error {
            return Err(format!(
                "No Android packages were found for `{device}`; running-process lookup failed after: {ps_error}"
            ));
        }
    }

    Ok(packages)
}

pub async fn list_packages(device: &str) -> Vec<String> {
    list_packages_strict(device).await.unwrap_or_default()
}

pub async fn get_pid_for_package_strict(
    device: &str,
    package: &str,
) -> Result<Option<u32>, String> {
    let command = adb_command(
        ["-s", device, "shell", "pidof", package],
        "resolving an Android package PID",
        ADB_DEVICE_QUERY_TIMEOUT,
    );
    let output = command.run().await?;

    if output.status.success() {
        return Ok(parse_pidof_output(&output.stdout));
    }

    if combine_command_output(&output).is_empty() {
        return Ok(None);
    }

    Err(command.status_error(&output))
}

pub async fn get_pid_for_package(device: &str, package: &str) -> Option<u32> {
    get_pid_for_package_strict(device, package)
        .await
        .ok()
        .flatten()
}

/// Pair with a device using `adb pair host:port code`.
pub async fn pair_device(host_port: &str, code: &str) -> Result<String, String> {
    let command = adb_command(
        ["pair", host_port, code],
        "pairing an Android device over Wi-Fi",
        ADB_CONNECTION_TIMEOUT,
    );
    let output = command.run().await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let combined = combine_command_output(&output);

    if combined.to_lowercase().contains("success") {
        Ok(stdout)
    } else if output.status.success() {
        Err(if combined.is_empty() {
            format!(
                "`{}` completed while pairing an Android device over Wi-Fi but did not report success",
                command.display()
            )
        } else {
            format!(
                "`{}` completed while pairing an Android device over Wi-Fi but did not report success: {}",
                command.display(),
                combined
            )
        })
    } else {
        Err(command.status_error(&output))
    }
}

/// Connect to a device using `adb connect host:port`.
pub async fn connect_device(host_port: &str) -> Result<String, String> {
    let command = adb_command(
        ["connect", host_port],
        "connecting to an Android device over Wi-Fi",
        ADB_CONNECTION_TIMEOUT,
    );
    let output = command.run().await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let combined = combine_command_output(&output);

    if output.status.success() && combined.to_lowercase().contains("connected") {
        Ok(stdout)
    } else if output.status.success() {
        Err(if combined.is_empty() {
            format!(
                "`{}` completed while connecting to an Android device over Wi-Fi but did not report a connection",
                command.display()
            )
        } else {
            format!(
                "`{}` completed while connecting to an Android device over Wi-Fi but did not report a connection: {}",
                command.display(),
                combined
            )
        })
    } else {
        Err(command.status_error(&output))
    }
}

/// Disconnect a wireless device using `adb disconnect host:port`.
pub async fn disconnect_device(serial: &str) -> Result<String, String> {
    let command = adb_command(
        ["disconnect", serial],
        "disconnecting from an Android Wi-Fi device",
        ADB_CONNECTION_TIMEOUT,
    );
    let output = command.run().await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if output.status.success() || stdout.to_lowercase().contains("disconnected") {
        Ok(stdout)
    } else {
        Err(command.status_error(&output))
    }
}

/// Restart the adb server.
pub async fn restart_server() -> Result<String, String> {
    let kill_command = adb_command(
        ["kill-server"],
        "stopping the adb server",
        ADB_CONNECTION_TIMEOUT,
    );
    kill_command.ensure_success(kill_command.run().await?)?;

    let start_command = adb_command(
        ["start-server"],
        "starting the adb server",
        ADB_CONNECTION_TIMEOUT,
    );
    let output = start_command.ensure_success(start_command.run().await?)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if stdout.is_empty() {
        Ok("ADB server restarted".to_string())
    } else {
        Ok(stdout)
    }
}

/// Enable adb TCP/IP mode over USB and attempt to auto-connect to the device's Wi-Fi IP.
pub async fn enable_tcpip_mode(device: &str, port: u16) -> Result<TcpipEnableResult, String> {
    let port_string = port.to_string();
    let command = adb_command(
        ["-s", device, "tcpip", &port_string],
        "enabling adb TCP/IP mode",
        ADB_CONNECTION_TIMEOUT,
    );
    let output = command.ensure_success(command.run().await?)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

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
        message: format!("{prefix} Connect manually using the device Wi-Fi IP and port {port}."),
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
#[cfg(feature = "egui")]
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
                let _ =
                    tx.blocking_send(QrPairEvent::Finished(Err(format!("mDNS init failed: {e}"))));
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
                        if !connect_ports
                            .iter()
                            .any(|entry| entry == &(addr.clone(), port))
                        {
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

                    let pair_command = OneShotCommand::new(
                        adb_path.clone(),
                        vec!["pair".to_string(), pair_addr.clone(), password.clone()],
                        "pairing an Android device over Wi-Fi",
                        ADB_CONNECTION_TIMEOUT,
                    );
                    let output = match pair_command.run_blocking() {
                        Ok(output) => output,
                        Err(err) => {
                            eprintln!("adb pair command error: {err}");
                            let _ = tx.blocking_send(QrPairEvent::Finished(Err(err)));
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
                            let connect_command = OneShotCommand::new(
                                adb_path.clone(),
                                vec!["connect".to_string(), connect_addr.clone()],
                                "connecting to an Android device over Wi-Fi",
                                ADB_CONNECTION_TIMEOUT,
                            );
                            match connect_command.run_blocking() {
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
                                        let connect_error = if output.status.success() {
                                            connect_combined.trim().to_string()
                                        } else {
                                            connect_command.status_error(&output)
                                        };
                                        let _ =
                                            tx.blocking_send(QrPairEvent::Finished(Ok(format!(
                                                "Paired with {pair_addr}. Automatic connect to \
                                                 {connect_addr} failed: {}. Use the Connect \
                                                 section with the device's connect port if needed.",
                                                connect_error
                                            ))));
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
                        let pair_error = if output.status.success() {
                            combined.trim().to_string()
                        } else {
                            pair_command.status_error(&output)
                        };
                        eprintln!("Pair attempt with {} failed: {}", pair_addr, pair_error);
                        let _ = tx.blocking_send(QrPairEvent::Finished(Err(pair_error)));
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
    let command = adb_command(
        ["-s", device, "shell", "ip", "route"],
        "querying an Android device Wi-Fi IP",
        ADB_DEVICE_QUERY_TIMEOUT,
    );
    let output = command.ensure_success(command.run().await?)?;

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

const ADB_LOCATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Check if a device serial looks like an Android emulator.
pub fn is_emulator(serial: &str) -> bool {
    serial.starts_with("emulator-")
}

async fn run_emulator_console_command(
    serial: &str,
    args: &[&str],
    context: &'static str,
) -> Result<(), String> {
    let command = adb_command(
        ["-s", serial, "emu"]
            .into_iter()
            .chain(args.iter().copied()),
        context,
        ADB_NETWORK_TIMEOUT,
    );
    let output = command.run().await?;
    command.ensure_success(output)?;
    Ok(())
}

async fn run_android_shell_command(
    serial: &str,
    args: &[&str],
    context: &'static str,
) -> Result<(), String> {
    let command = adb_command(
        ["-s", serial, "shell"]
            .into_iter()
            .chain(args.iter().copied()),
        context,
        ADB_NETWORK_TIMEOUT,
    );
    let output = command.run().await?;
    command.ensure_success(output)?;
    Ok(())
}

/// Set the GPS location on an Android emulator.
/// Only works on emulator devices (serial starting with "emulator-").
/// The `adb emu geo fix` command takes longitude first, then latitude.
pub async fn set_emulator_location(
    serial: &str,
    lat: f64,
    lon: f64,
    alt: Option<f64>,
) -> Result<String, String> {
    if !is_emulator(serial) {
        return Err(format!(
            "Location spoofing via adb is only supported on emulators, not physical device '{}'",
            serial
        ));
    }
    // adb emu geo fix <longitude> <latitude> [<altitude>]
    let mut args = vec![
        "-s".to_string(),
        serial.to_string(),
        "emu".to_string(),
        "geo".to_string(),
        "fix".to_string(),
        format!("{}", lon),
        format!("{}", lat),
    ];
    if let Some(altitude) = alt {
        args.push(format!("{}", altitude));
    }

    let cmd = adb_command(args, "set emulator location", ADB_LOCATION_TIMEOUT);
    let output = cmd.run().await?;
    cmd.ensure_success(output)?;
    Ok(format!("Location set to {}, {} on {}", lat, lon, serial))
}

pub async fn apply_emulator_network_condition(
    serial: &str,
    preset: NetworkConditionPreset,
) -> Result<String, String> {
    if !is_emulator(serial) {
        return Err(format!(
            "Network throttling via adb is only supported on emulators, not physical device '{}'",
            serial
        ));
    }

    let profile = preset.android_profile();
    run_emulator_console_command(
        serial,
        &["network", "speed", profile.speed],
        "setting emulator network speed",
    )
    .await?;
    run_emulator_console_command(
        serial,
        &["network", "delay", profile.delay],
        "setting emulator network delay",
    )
    .await?;

    if profile.data_enabled {
        run_emulator_console_command(
            serial,
            &["gsm", "data", "on"],
            "enabling emulator cellular data",
        )
        .await?;
    } else {
        run_emulator_console_command(
            serial,
            &["gsm", "data", "off"],
            "disabling emulator cellular data",
        )
        .await?;
    }

    if profile.wifi_enabled {
        run_android_shell_command(
            serial,
            &["svc", "wifi", "enable"],
            "enabling emulator Wi-Fi",
        )
        .await?;
    } else {
        run_android_shell_command(
            serial,
            &["svc", "wifi", "disable"],
            "disabling emulator Wi-Fi",
        )
        .await?;
    }

    Ok(format!(
        "Applied {} network condition on {}",
        preset.label(),
        serial
    ))
}

pub async fn clear_emulator_network_condition(serial: &str) -> Result<String, String> {
    apply_emulator_network_condition(serial, NetworkConditionPreset::Unthrottled).await?;
    Ok(format!("Cleared network throttling on {}", serial))
}

/// Dispatch a [`NetworkConditionSpec`] to the right Android backend.
///
/// * **Emulators** continue to use the existing `adb emu network …` console
///   commands. Custom shaping params are not supported by the emulator
///   console, so they're rejected up front with a clear message; preset
///   specs are applied as before.
/// * **Physical devices** route through the CatPane helper app
///   ([`crate::throttle_android`]) over an `adb forward`-backed control
///   socket. Both presets and custom params are supported.
pub async fn apply_android_network_condition(
    serial: &str,
    spec: crate::network_condition::NetworkConditionSpec,
) -> Result<String, String> {
    spec.validate()?;
    if is_emulator(serial) {
        match spec {
            crate::network_condition::NetworkConditionSpec::Preset { preset } => {
                apply_emulator_network_condition(serial, preset).await
            }
            crate::network_condition::NetworkConditionSpec::Custom { .. } => Err(
                "Custom network shaping parameters are not supported on Android emulators. \
                 Pick a preset (unthrottled / edge / 3g / offline) or use a physical device \
                 with the CatPane helper app installed."
                    .to_string(),
            ),
        }
    } else {
        crate::throttle_android::apply_device_network_condition(serial, spec).await
    }
}

/// Dispatch a "clear throttling" request to the right Android backend.
pub async fn clear_android_network_condition(serial: &str) -> Result<String, String> {
    if is_emulator(serial) {
        clear_emulator_network_condition(serial).await
    } else {
        crate::throttle_android::clear_device_network_condition(serial).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_ip_from_route_output, parse_adb_devices_output, parse_pidof_output,
        parse_running_packages_output, service_matches_expected_name,
    };
    use crate::network_condition::NetworkConditionPreset;

    #[test]
    fn parses_and_deduplicates_adb_devices() {
        let devices = parse_adb_devices_output(
            br#"List of devices attached
usb-serial device product:oriole model:Pixel_6 device:oriole transport_id:1
adb-123._adb-tls-connect._tcp device product:oriole model:Pixel_6 device:oriole transport_id:2
192.168.1.25:5555 device product:oriole model:Pixel_6 device:oriole transport_id:3
"#,
        );

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "192.168.1.25:5555");
    }

    #[test]
    fn parses_running_package_names() {
        let packages = parse_running_packages_output(
            b"NAME\n/system/bin/sh\ncom.example.one\n[com.android.shell]\ncom.example.two\n",
        );

        assert_eq!(packages, vec!["com.example.one", "com.example.two"]);
    }

    #[test]
    fn parses_first_pid_from_pidof_output() {
        assert_eq!(parse_pidof_output(b"1234 5678\n"), Some(1234));
    }

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

    #[test]
    fn detects_android_emulators_by_serial() {
        assert!(super::is_emulator("emulator-5554"));
        assert!(!super::is_emulator("R58M12345AB"));
    }

    #[test]
    fn offline_profile_disables_transport_flags() {
        let profile = NetworkConditionPreset::Offline.android_profile();
        assert!(!profile.data_enabled);
        assert!(!profile.wifi_enabled);
    }
}
