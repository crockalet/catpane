use std::process::Stdio;

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{mpsc, watch},
};

use crate::{
    adb, ios, ios_device,
    ios_noise::{IOS_SYSTEM_PROCESSES, IOS_SYSTEM_SUBSYSTEM_PREFIXES},
    log_buffer_config::initial_log_backlog,
    log_entry::{LogEntry, parse_ios_log_ndjson_line, parse_ios_syslog_line, parse_logcat_line},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureScope {
    pub process: Option<String>,
    pub text: Option<String>,
    pub predicate: Option<String>,
    pub quiet: bool,
    pub clean: bool,
}

impl CaptureScope {
    pub fn is_empty(&self) -> bool {
        self.process.is_none()
            && self.text.is_none()
            && self.predicate.is_none()
            && !self.quiet
            && !self.clean
    }

    pub fn is_explicitly_scoped(&self) -> bool {
        self.process.is_some() || self.text.is_some() || self.predicate.is_some()
    }
}

pub fn default_clean_capture(platform: DevicePlatform) -> bool {
    matches!(
        platform,
        DevicePlatform::IosDevice | DevicePlatform::IosSimulator
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DevicePlatform {
    Android,
    IosDevice,
    IosSimulator,
}

impl DevicePlatform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::IosDevice => "iOS Device",
            Self::IosSimulator => "iOS Simulator",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedDevice {
    pub id: String,
    pub name: String,
    pub description: String,
    pub platform: DevicePlatform,
}

impl std::fmt::Display for ConnectedDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl ConnectedDevice {
    pub fn display_name(&self) -> String {
        format!("{} ({})", self.name, self.platform.label())
    }

    pub fn supports_package_filter(&self) -> bool {
        self.platform == DevicePlatform::Android
    }

    pub fn supports_ios_filters(&self) -> bool {
        matches!(
            self.platform,
            DevicePlatform::IosDevice | DevicePlatform::IosSimulator
        )
    }

    pub fn supports_wireless_debugging(&self) -> bool {
        self.platform == DevicePlatform::Android
    }

    pub fn supports_disconnect(&self) -> bool {
        self.platform == DevicePlatform::Android && adb::is_tcp_device(&self.id)
    }
}

#[derive(Debug, Clone)]
pub struct CaptureController {
    kill_tx: mpsc::Sender<()>,
    completion_rx: watch::Receiver<bool>,
}

impl CaptureController {
    pub fn stop(&self) {
        let _ = self.kill_tx.try_send(());
    }

    pub async fn wait_for_shutdown(&self) {
        wait_for_completion(self.completion_rx.clone()).await;
    }

    #[doc(hidden)]
    pub fn test_controller() -> (Self, mpsc::Receiver<()>, watch::Sender<bool>) {
        let (kill_tx, kill_rx) = mpsc::channel(1);
        let (completion_tx, completion_rx) = watch::channel(false);
        (
            Self {
                kill_tx,
                completion_rx,
            },
            kill_rx,
            completion_tx,
        )
    }
}

pub struct CaptureHandle {
    pub rx: mpsc::Receiver<LogEntry>,
    controller: CaptureController,
}

impl CaptureHandle {
    pub fn stop(&self) {
        self.controller.stop();
    }

    pub fn controller(&self) -> CaptureController {
        self.controller.clone()
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.controller.stop();
    }
}

pub async fn list_devices_strict() -> Result<Vec<ConnectedDevice>, String> {
    let mut devices: Vec<ConnectedDevice> = adb::list_devices_strict()
        .await?
        .into_iter()
        .map(|device| ConnectedDevice {
            id: device.serial.clone(),
            name: device.friendly_name(),
            description: device.description,
            platform: DevicePlatform::Android,
        })
        .collect();

    devices.extend(
        ios_device::list_connected_devices_strict()
            .await?
            .into_iter()
            .map(|device| ConnectedDevice {
                id: device.udid,
                name: device.name,
                description: device.description,
                platform: DevicePlatform::IosDevice,
            }),
    );

    devices.extend(
        ios::list_booted_simulators_strict()
            .await?
            .into_iter()
            .map(|sim| ConnectedDevice {
                id: sim.udid,
                name: sim.name,
                description: sim.runtime,
                platform: DevicePlatform::IosSimulator,
            }),
    );

    devices.sort_by(|left, right| {
        left.platform
            .label()
            .cmp(right.platform.label())
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(devices)
}

pub async fn list_devices() -> Vec<ConnectedDevice> {
    let mut devices: Vec<ConnectedDevice> = adb::list_devices()
        .await
        .into_iter()
        .map(|device| ConnectedDevice {
            id: device.serial.clone(),
            name: device.friendly_name(),
            description: device.description,
            platform: DevicePlatform::Android,
        })
        .collect();

    devices.extend(
        ios_device::list_connected_devices()
            .await
            .into_iter()
            .map(|device| ConnectedDevice {
                id: device.udid,
                name: device.name,
                description: device.description,
                platform: DevicePlatform::IosDevice,
            }),
    );

    devices.extend(
        ios::list_booted_simulators()
            .await
            .into_iter()
            .map(|sim| ConnectedDevice {
                id: sim.udid,
                name: sim.name,
                description: sim.runtime,
                platform: DevicePlatform::IosSimulator,
            }),
    );

    devices.sort_by(|left, right| {
        left.platform
            .label()
            .cmp(right.platform.label())
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    devices
}

pub fn spawn_device_tracker(rt: &tokio::runtime::Handle) -> mpsc::Receiver<Vec<ConnectedDevice>> {
    let (tx, rx) = mpsc::channel::<Vec<ConnectedDevice>>(4);

    rt.spawn(async move {
        let mut previous = Vec::new();
        loop {
            let devices = list_devices().await;
            if devices != previous {
                previous = devices.clone();
                if tx.send(devices).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });

    rx
}

#[allow(dead_code)]
pub async fn list_packages_strict(
    device_id: &str,
    devices: &[ConnectedDevice],
) -> Result<Vec<String>, String> {
    match devices.iter().find(|device| device.id == device_id) {
        Some(device) if device.platform == DevicePlatform::Android => {
            adb::list_packages_strict(device_id).await
        }
        _ => Ok(Vec::new()),
    }
}

pub async fn list_packages(device_id: &str, devices: &[ConnectedDevice]) -> Vec<String> {
    match devices.iter().find(|device| device.id == device_id) {
        Some(device) if device.platform == DevicePlatform::Android => {
            adb::list_packages(device_id).await
        }
        _ => Vec::new(),
    }
}

#[allow(dead_code)]
pub async fn get_pid_for_package_strict(
    device_id: &str,
    package: &str,
    devices: &[ConnectedDevice],
) -> Result<Option<u32>, String> {
    match devices.iter().find(|device| device.id == device_id) {
        Some(device) if device.platform == DevicePlatform::Android => {
            adb::get_pid_for_package_strict(device_id, package).await
        }
        _ => Ok(None),
    }
}

pub async fn get_pid_for_package(
    device_id: &str,
    package: &str,
    devices: &[ConnectedDevice],
) -> Option<u32> {
    match devices.iter().find(|device| device.id == device_id) {
        Some(device) if device.platform == DevicePlatform::Android => {
            adb::get_pid_for_package(device_id, package).await
        }
        _ => None,
    }
}

pub fn spawn_capture(
    rt: &tokio::runtime::Handle,
    device: &ConnectedDevice,
    pid_filter: Option<u32>,
    scope: CaptureScope,
) -> CaptureHandle {
    match device.platform {
        DevicePlatform::Android => spawn_android_capture(rt, device.id.clone(), pid_filter),
        DevicePlatform::IosDevice => spawn_ios_device_capture(rt, device.id.clone(), scope),
        DevicePlatform::IosSimulator => spawn_ios_simulator_capture(rt, device.id.clone(), scope),
    }
}

fn spawn_android_capture(
    rt: &tokio::runtime::Handle,
    device_id: String,
    pid_filter: Option<u32>,
) -> CaptureHandle {
    let (tx, rx) = mpsc::channel::<LogEntry>(4096);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (completion_tx, completion_rx) = watch::channel(false);
    let controller = CaptureController {
        kill_tx,
        completion_rx,
    };

    rt.spawn(async move {
        let _completion = CompletionSignal::new(completion_tx);
        let mut args = vec![
            "-s".to_string(),
            device_id,
            "logcat".to_string(),
            "-T".to_string(),
            initial_log_backlog().to_string(),
            "-v".to_string(),
            "threadtime".to_string(),
        ];
        if let Some(pid) = pid_filter {
            args.push(format!("--pid={pid}"));
        }

        let mut child = match Command::new(adb::adb_binary())
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return,
        };

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return,
        };

        let mut reader = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                line = reader.next_line() => match line {
                    Ok(Some(line)) => {
                        if let Some(entry) = parse_logcat_line(&line) {
                            if tx.send(entry).await.is_err() {
                                break;
                            }
                        }
                    }
                    _ => break,
                },
                _ = kill_rx.recv() => break,
            }
        }
        drop(reader);
        let _ = child.kill().await;
    });

    CaptureHandle { rx, controller }
}

fn spawn_ios_simulator_capture(
    rt: &tokio::runtime::Handle,
    udid: String,
    scope: CaptureScope,
) -> CaptureHandle {
    let (tx, rx) = mpsc::channel::<LogEntry>(4096);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (completion_tx, completion_rx) = watch::channel(false);
    let controller = CaptureController {
        kill_tx,
        completion_rx,
    };

    rt.spawn(async move {
        let _completion = CompletionSignal::new(completion_tx);
        let args = build_ios_simulator_capture_args(&udid, &scope);
        let mut child = match Command::new("xcrun")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return,
        };

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return,
        };

        let mut reader = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                line = reader.next_line() => match line {
                    Ok(Some(line)) => {
                        if let Some(entry) = parse_ios_log_ndjson_line(&line) {
                            if tx.send(entry).await.is_err() {
                                break;
                            }
                        }
                    }
                    _ => break,
                },
                _ = kill_rx.recv() => break,
            }
        }
        drop(reader);
        let _ = child.kill().await;
    });

    CaptureHandle { rx, controller }
}

fn spawn_ios_device_capture(
    rt: &tokio::runtime::Handle,
    udid: String,
    scope: CaptureScope,
) -> CaptureHandle {
    let (tx, rx) = mpsc::channel::<LogEntry>(4096);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (completion_tx, completion_rx) = watch::channel(false);
    let controller = CaptureController {
        kill_tx,
        completion_rx,
    };

    rt.spawn(async move {
        let _completion = CompletionSignal::new(completion_tx);
        let args = build_ios_device_capture_args(&udid, &scope);
        let mut child = match Command::new(ios_device::idevicesyslog_binary())
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return,
        };

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return,
        };

        let mut reader = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                line = reader.next_line() => match line {
                    Ok(Some(line)) => {
                        if let Some(entry) = parse_ios_syslog_line(&line) {
                            if tx.send(entry).await.is_err() {
                                break;
                            }
                        }
                    }
                    _ => break,
                },
                _ = kill_rx.recv() => break,
            }
        }
        drop(reader);
        let _ = child.kill().await;
    });

    CaptureHandle { rx, controller }
}

fn build_ios_simulator_capture_args(udid: &str, scope: &CaptureScope) -> Vec<String> {
    let mut args = vec![
        "simctl".to_string(),
        "spawn".to_string(),
        udid.to_string(),
        "log".to_string(),
        "stream".to_string(),
        "--style".to_string(),
        "ndjson".to_string(),
        "--level".to_string(),
        "debug".to_string(),
    ];

    if let Some(predicate) = build_ios_simulator_predicate(scope) {
        args.push("--predicate".to_string());
        args.push(predicate);
    } else if let Some(process) = scope.process.as_deref() {
        args.push("--process".to_string());
        args.push(process.to_string());
    }

    args
}

fn build_ios_simulator_predicate(scope: &CaptureScope) -> Option<String> {
    let mut clauses = Vec::new();

    if scope.clean {
        clauses.push(simulator_clean_predicate_clause());
    }
    if (scope.text.is_some() || scope.predicate.is_some())
        && let Some(process) = scope.process.as_deref()
    {
        clauses.push(format!("process == {}", quote_predicate_string(process)));
    }
    if let Some(text) = scope.text.as_deref() {
        clauses.push(format!(
            "composedMessage CONTAINS[c] {}",
            quote_predicate_string(text)
        ));
    }
    if let Some(predicate) = scope.predicate.as_deref() {
        clauses.push(format!("({predicate})"));
    }

    (!clauses.is_empty()).then(|| clauses.join(" AND "))
}

fn build_ios_device_capture_args(udid: &str, scope: &CaptureScope) -> Vec<String> {
    let mut args = vec!["-u".to_string(), udid.to_string()];

    if scope.clean || scope.quiet {
        args.push("--quiet".to_string());
    }
    if let Some(process) = scope.process.as_deref() {
        args.push("--process".to_string());
        args.push(process.to_string());
    }
    if let Some(text) = scope.text.as_deref() {
        args.push("--match".to_string());
        args.push(text.to_string());
    }

    args
}

fn quote_predicate_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn simulator_clean_predicate_clause() -> String {
    let process_clause = IOS_SYSTEM_PROCESSES
        .iter()
        .map(|process| format!("process == {}", quote_predicate_string(process)))
        .collect::<Vec<_>>()
        .join(" OR ");
    let subsystem_clause = IOS_SYSTEM_SUBSYSTEM_PREFIXES
        .iter()
        .map(|prefix| format!("subsystem BEGINSWITH {}", quote_predicate_string(prefix)))
        .collect::<Vec<_>>()
        .join(" OR ");

    format!("NOT (({process_clause}) OR ({subsystem_clause}))")
}

struct CompletionSignal {
    tx: Option<watch::Sender<bool>>,
}

impl CompletionSignal {
    fn new(tx: watch::Sender<bool>) -> Self {
        Self { tx: Some(tx) }
    }
}

impl Drop for CompletionSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(true);
        }
    }
}

async fn wait_for_completion(mut completion_rx: watch::Receiver<bool>) {
    if *completion_rx.borrow() {
        return;
    }

    while completion_rx.changed().await.is_ok() {
        if *completion_rx.borrow() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulator_process_scope_uses_process_flag() {
        let args = build_ios_simulator_capture_args(
            "SIM-1",
            &CaptureScope {
                process: Some("MyApp".into()),
                ..CaptureScope::default()
            },
        );

        assert_eq!(
            args,
            vec![
                "simctl",
                "spawn",
                "SIM-1",
                "log",
                "stream",
                "--style",
                "ndjson",
                "--level",
                "debug",
                "--process",
                "MyApp",
            ]
        );
    }

    #[test]
    fn simulator_text_and_process_scope_builds_predicate() {
        let args = build_ios_simulator_capture_args(
            "SIM-1",
            &CaptureScope {
                process: Some("MyApp".into()),
                text: Some("timeout".into()),
                ..CaptureScope::default()
            },
        );

        assert_eq!(
            args,
            vec![
                "simctl",
                "spawn",
                "SIM-1",
                "log",
                "stream",
                "--style",
                "ndjson",
                "--level",
                "debug",
                "--predicate",
                "process == \"MyApp\" AND composedMessage CONTAINS[c] \"timeout\"",
            ]
        );
    }

    #[test]
    fn simulator_clean_scope_builds_exclusion_predicate() {
        let args = build_ios_simulator_capture_args(
            "SIM-1",
            &CaptureScope {
                clean: true,
                ..CaptureScope::default()
            },
        );

        assert_eq!(
            args[..9],
            [
                "simctl", "spawn", "SIM-1", "log", "stream", "--style", "ndjson", "--level",
                "debug"
            ]
        );
        assert_eq!(args[9], "--predicate");
        assert!(args[10].contains("NOT (("));
        assert!(args[10].contains("process == \"SpringBoard\""));
        assert!(args[10].contains("subsystem BEGINSWITH \"com.apple.\""));
    }

    #[test]
    fn simulator_user_predicate_is_combined_with_generated_filters() {
        let args = build_ios_simulator_capture_args(
            "SIM-1",
            &CaptureScope {
                process: Some("MyApp".into()),
                predicate: Some("subsystem == \"com.example.app\"".into()),
                ..CaptureScope::default()
            },
        );

        assert_eq!(
            args.last().map(String::as_str),
            Some("process == \"MyApp\" AND (subsystem == \"com.example.app\")")
        );
    }

    #[test]
    fn device_scope_uses_idevicesyslog_filters() {
        let args = build_ios_device_capture_args(
            "DEVICE-1",
            &CaptureScope {
                process: Some("MyApp".into()),
                text: Some("timeout".into()),
                predicate: Some("ignored".into()),
                quiet: true,
                ..CaptureScope::default()
            },
        );

        assert_eq!(
            args,
            vec![
                "-u",
                "DEVICE-1",
                "--quiet",
                "--process",
                "MyApp",
                "--match",
                "timeout",
            ]
        );
    }

    #[test]
    fn device_clean_scope_enables_quiet() {
        let args = build_ios_device_capture_args(
            "DEVICE-1",
            &CaptureScope {
                clean: true,
                ..CaptureScope::default()
            },
        );

        assert_eq!(args, vec!["-u", "DEVICE-1", "--quiet"]);
    }

    #[test]
    fn predicate_escaping_handles_quotes_and_backslashes() {
        assert_eq!(
            quote_predicate_string("say \"hi\" \\ now"),
            "\"say \\\"hi\\\" \\\\ now\""
        );
    }
}
