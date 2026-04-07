use std::process::Stdio;

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{mpsc, watch},
};

use crate::{
    adb, ios,
    log_buffer_config::log_buffer_capacity,
    log_entry::{LogEntry, LogPlatform, parse_ios_log_ndjson_line, parse_logcat_line},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DevicePlatform {
    Android,
    IosSimulator,
}

impl DevicePlatform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::IosSimulator => "iOS Simulator",
        }
    }
}

impl From<LogPlatform> for DevicePlatform {
    fn from(value: LogPlatform) -> Self {
        match value {
            LogPlatform::Android => Self::Android,
            LogPlatform::IosSimulator => Self::IosSimulator,
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
        self.platform == DevicePlatform::IosSimulator
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

    /// Create a test controller with access to the raw kill/completion channels.
    /// Useful for testing code that interacts with capture lifecycle.
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

    devices.extend(ios::list_booted_simulators().await.into_iter().map(|sim| {
        ConnectedDevice {
            id: sim.udid,
            name: sim.name,
            description: sim.runtime,
            platform: DevicePlatform::IosSimulator,
        }
    }));

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
) -> CaptureHandle {
    match device.platform {
        DevicePlatform::Android => spawn_android_capture(rt, device.id.clone(), pid_filter),
        DevicePlatform::IosSimulator => spawn_ios_simulator_capture(rt, device.id.clone()),
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
            log_buffer_capacity().to_string(),
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
                _ = kill_rx.recv() => {
                    let _ = child.kill().await;
                    break;
                }
            }
        }
    });

    CaptureHandle { rx, controller }
}

fn spawn_ios_simulator_capture(rt: &tokio::runtime::Handle, udid: String) -> CaptureHandle {
    let (tx, rx) = mpsc::channel::<LogEntry>(4096);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (completion_tx, completion_rx) = watch::channel(false);
    let controller = CaptureController {
        kill_tx,
        completion_rx,
    };

    rt.spawn(async move {
        let _completion = CompletionSignal::new(completion_tx);
        let mut child = match Command::new("xcrun")
            .args([
                "simctl", "spawn", &udid, "log", "stream", "--style", "ndjson", "--level", "debug",
            ])
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
                _ = kill_rx.recv() => {
                    let _ = child.kill().await;
                    break;
                }
            }
        }
    });

    CaptureHandle { rx, controller }
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
