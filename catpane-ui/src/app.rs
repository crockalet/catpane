use crate::pane::{Pane, PaneId, PaneNode, SplitDir, default_word_wrap};
use catpane_core::capture::{self, CaptureController, ConnectedDevice};
use catpane_core::log_entry::{LogEntry, LogLevel};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::{sync::broadcast, task::JoinHandle};

use egui;

const TAG_HISTORY_MAX: usize = 50;

struct SharedCapture {
    tx: broadcast::Sender<LogEntry>,
    controller: CaptureController,
    fanout_task: JoinHandle<()>,
    subscriber_count: usize,
}

fn default_sidebar_open() -> bool {
    false
}

pub struct QrPairingState {
    pub qr_texture: Option<egui::TextureHandle>,
    pub mdns_rx: tokio::sync::mpsc::Receiver<catpane_core::adb::QrPairEvent>,
    pub status: QrPairStatus,
}

#[derive(Clone, Debug)]
pub enum QrPairStatus {
    WaitingScan,
    Pairing(String),
    Success(String),
    Failed(String),
}

// Serializable session state
#[derive(Serialize, Deserialize)]
struct SessionPane {
    device: Option<String>,
    package: Option<String>,
    #[serde(default)]
    ios_process: String,
    #[serde(default)]
    ios_subsystem: String,
    #[serde(default)]
    ios_category: String,
    tag_input: String,
    min_level: LogLevel,
    hide_vendor_noise: bool,
    #[serde(default = "default_word_wrap")]
    word_wrap: bool,
}

#[derive(Serialize, Deserialize)]
enum SessionTree {
    Leaf(usize), // index into panes vec
    Split {
        dir: String,
        first: Box<SessionTree>,
        second: Box<SessionTree>,
    },
}

#[derive(Serialize, Deserialize)]
struct Session {
    panes: Vec<SessionPane>,
    tree: SessionTree,
    focused: usize,
    #[serde(default = "default_sidebar_open")]
    sidebar_open: bool,
}

pub struct App {
    pub pane_tree: PaneNode,
    pub panes: HashMap<PaneId, Pane>,
    pub focused_pane: PaneId,
    pub sidebar_open: bool,
    pub devices: Vec<ConnectedDevice>,
    pub rt: tokio::runtime::Handle,
    pub show_help: bool,
    pub device_refresh_pending: bool,
    pub ios_simulator_refresh_pending: bool,
    pub tag_history: Vec<String>,
    shared_captures: HashMap<String, SharedCapture>,
    pub device_tracker: Option<tokio::sync::mpsc::Receiver<Vec<ConnectedDevice>>>,
    // Wireless debugging state
    pub wireless_pair_host: String,
    pub wireless_pair_code: String,
    pub wireless_connect_host: String,
    pub wireless_status: Option<(bool, String)>,
    pub wireless_usb_device: Option<String>,
    // QR pairing state
    pub qr_pairing: Option<QrPairingState>,
    // iOS simulator state
    pub ios_simulators: Vec<catpane_core::ios::IosSimulator>,
    pub ios_simulator_status: Option<(bool, String)>,
    pub ios_simulator_booting_udid: Option<String>,
    pub ios_simulator_boot_rx: Option<tokio::sync::mpsc::Receiver<Result<String, String>>>,
    // Location spoofing state
    pub location_lat: String,
    pub location_lon: String,
    pub location_preset: String,
    pub location_status: Option<(bool, String)>,
    pub location_rx: Option<tokio::sync::mpsc::Receiver<Result<String, String>>>,
}

impl App {
    pub fn new(rt: tokio::runtime::Handle, devices: Vec<ConnectedDevice>) -> Self {
        // Try restoring session
        if let Some(app) = Self::try_restore(&rt, &devices) {
            return app;
        }

        let default_device = devices.first().map(|d| d.id.clone());
        let pane = Pane::new(default_device);
        let id = pane.id;

        let mut panes = HashMap::new();
        panes.insert(id, pane);

        let device_tracker = Some(capture::spawn_device_tracker(&rt));

        let mut app = Self {
            pane_tree: PaneNode::leaf(id),
            panes,
            focused_pane: id,
            sidebar_open: default_sidebar_open(),
            devices,
            rt,
            show_help: false,
            device_refresh_pending: false,
            ios_simulator_refresh_pending: false,
            tag_history: Self::load_tag_history(),
            shared_captures: HashMap::new(),
            device_tracker,
            wireless_pair_host: catpane_core::adb::local_ip_prefix(),
            wireless_pair_code: String::new(),
            wireless_connect_host: catpane_core::adb::local_ip_prefix(),
            wireless_status: None,
            wireless_usb_device: None,
            qr_pairing: None,
            ios_simulators: Vec::new(),
            ios_simulator_status: None,
            ios_simulator_booting_udid: None,
            ios_simulator_boot_rx: None,
            location_lat: String::new(),
            location_lon: String::new(),
            location_preset: "Custom".to_string(),
            location_status: None,
            location_rx: None,
        };
        app.ensure_pane_capture(id);
        app
    }

    pub fn poll_all(&mut self) {
        let ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for id in ids {
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.ingest_lines();
            }
        }
    }

    pub fn poll_qr_pairing(&mut self) {
        let mut should_refresh_devices = false;

        if let Some(qr) = &mut self.qr_pairing {
            if matches!(
                qr.status,
                QrPairStatus::WaitingScan | QrPairStatus::Pairing(_)
            ) {
                while let Ok(event) = qr.mdns_rx.try_recv() {
                    match event {
                        catpane_core::adb::QrPairEvent::Status(msg) => {
                            qr.status = QrPairStatus::Pairing(msg);
                        }
                        catpane_core::adb::QrPairEvent::Finished(Ok(msg)) => {
                            qr.status = QrPairStatus::Success(msg);
                            should_refresh_devices = true;
                            break;
                        }
                        catpane_core::adb::QrPairEvent::Finished(Err(msg)) => {
                            qr.status = QrPairStatus::Failed(msg);
                            break;
                        }
                    }
                }
            }
        }

        if should_refresh_devices {
            self.device_refresh_pending = true;
        }
    }

    pub fn needs_live_repaint(&self) -> bool {
        if self.device_refresh_pending
            || self.ios_simulator_refresh_pending
            || self.ios_simulator_boot_rx.is_some()
            || self.ios_simulator_booting_udid.is_some()
            || self.location_rx.is_some()
        {
            return true;
        }

        if self.qr_pairing.as_ref().is_some_and(|qr| {
            matches!(
                qr.status,
                QrPairStatus::WaitingScan | QrPairStatus::Pairing(_)
            )
        }) {
            return true;
        }

        self.panes
            .values()
            .any(|pane| pane.capture_rx.is_some() && !pane.paused)
    }

    pub fn set_focused_pane_device(&mut self, device_id: Option<String>) {
        self.set_pane_device(self.focused_pane, device_id);
    }

    pub fn set_pane_device(&mut self, pane_id: PaneId, device_id: Option<String>) {
        let current_device = self
            .panes
            .get(&pane_id)
            .and_then(|pane| pane.device.clone());
        if current_device == device_id {
            return;
        }

        self.detach_pane_capture(pane_id);

        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return;
        };

        pane.clear();
        pane.device = device_id;
        pane.pid_filter = None;
        pane.filter.package = None;
        pane.filter.ios_process = None;
        pane.filter.ios_subsystem = None;
        pane.filter.ios_category = None;
        pane.packages.clear();
        pane.package_refresh_pending = false;
        pane.package_filter_text.clear();
        pane.ios_process_filter_text.clear();
        pane.ios_subsystem_filter_text.clear();
        pane.ios_category_filter_text.clear();
        self.ensure_pane_capture(pane_id);
    }

    pub fn split_pane(&mut self, dir: SplitDir) {
        // Limit split depth to 2 (max 4 panes)
        let current_depth = self.pane_tree.depth_of(self.focused_pane).unwrap_or(0);
        if current_depth >= 2 {
            return;
        }

        let device = self
            .panes
            .get(&self.focused_pane)
            .and_then(|p| p.device.clone())
            .or_else(|| self.devices.first().map(|d| d.id.clone()));

        let new_pane = Pane::new(device);
        let new_id = new_pane.id;

        self.pane_tree.split(self.focused_pane, dir, new_id);
        self.panes.insert(new_id, new_pane);
        self.focused_pane = new_id;
        self.ensure_pane_capture(new_id);
    }

    pub fn close_pane(&mut self, id: PaneId) {
        if self.pane_tree.count() <= 1 {
            return;
        }
        self.detach_pane_capture(id);
        self.panes.remove(&id);
        self.pane_tree.remove(id);
        let ids = self.pane_tree.pane_ids();
        if !ids.contains(&self.focused_pane) {
            self.focused_pane = ids.first().copied().unwrap_or(0);
        }
    }

    pub fn cycle_focus(&mut self) {
        let ids = self.pane_tree.pane_ids();
        if ids.is_empty() {
            return;
        }
        let pos = ids
            .iter()
            .position(|&id| id == self.focused_pane)
            .unwrap_or(0);
        self.focused_pane = ids[(pos + 1) % ids.len()];
    }

    pub fn spawn_new_window(&self) {
        let exe = std::env::current_exe().unwrap_or_default();

        #[cfg(target_os = "macos")]
        {
            if let Some(app_bundle) = macos_app_bundle_for_executable(&exe) {
                let _ = std::process::Command::new("open")
                    .args(["-na"])
                    .arg(app_bundle)
                    .spawn();
            } else {
                let _ = std::process::Command::new(&exe).spawn();
            }
        }

        #[cfg(target_os = "linux")]
        {
            let exe_str = exe.to_string_lossy().to_string();
            for term in &["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
                if std::process::Command::new(term)
                    .args(["-e", &exe_str])
                    .spawn()
                    .is_ok()
                {
                    break;
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let exe_str = exe.to_string_lossy().to_string();
            let _ = std::process::Command::new("cmd")
                .args(["/c", "start", &exe_str])
                .spawn();
        }
    }

    pub fn ensure_pane_capture(&mut self, pane_id: PaneId) {
        let Some(device_id) = self
            .panes
            .get(&pane_id)
            .and_then(|pane| pane.device.clone())
        else {
            self.detach_pane_capture(pane_id);
            return;
        };

        let already_attached = self.panes.get(&pane_id).is_some_and(|pane| {
            pane.capture_rx.is_some()
                && pane.capture_device_id.as_deref() == Some(device_id.as_str())
        });
        if already_attached {
            return;
        }

        self.detach_pane_capture(pane_id);
        if !self.ensure_shared_capture(&device_id) {
            return;
        }

        let capture_rx = {
            let shared = self
                .shared_captures
                .get_mut(&device_id)
                .expect("shared capture exists");
            shared.subscriber_count += 1;
            shared.tx.subscribe()
        };

        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.start_capture(device_id, capture_rx);
        }
    }

    fn ensure_shared_capture(&mut self, device_id: &str) -> bool {
        let should_recreate = self
            .shared_captures
            .get(device_id)
            .is_some_and(|shared| shared.fanout_task.is_finished());
        if should_recreate {
            self.remove_shared_capture(device_id);
        }

        if self.shared_captures.contains_key(device_id) {
            return true;
        }

        let Some(device) = self
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .cloned()
        else {
            return false;
        };

        let mut handle = capture::spawn_capture(&self.rt, &device, None);
        let controller = handle.controller();
        let (tx, _) = broadcast::channel::<LogEntry>(4096);
        let fanout_tx = tx.clone();
        let fanout_task = self.rt.spawn(async move {
            while let Some(entry) = handle.rx.recv().await {
                let _ = fanout_tx.send(entry);
            }
        });

        self.shared_captures.insert(
            device_id.to_string(),
            SharedCapture {
                tx,
                controller,
                fanout_task,
                subscriber_count: 0,
            },
        );
        true
    }

    fn detach_pane_capture(&mut self, pane_id: PaneId) {
        let capture_device_id = self
            .panes
            .get(&pane_id)
            .and_then(|pane| pane.capture_device_id.clone());

        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.stop_capture();
        }

        let Some(device_id) = capture_device_id else {
            return;
        };

        let mut should_remove = false;
        if let Some(shared) = self.shared_captures.get_mut(&device_id) {
            shared.subscriber_count = shared.subscriber_count.saturating_sub(1);
            should_remove = shared.subscriber_count == 0;
        }
        if should_remove {
            self.remove_shared_capture(&device_id);
        }
    }

    fn remove_shared_capture(&mut self, device_id: &str) {
        if let Some(shared) = self.shared_captures.remove(device_id) {
            shared.controller.stop();
            shared.fanout_task.abort();
        }
    }

    pub fn save_tag_to_history(&mut self, tag_expr: &str) {
        let tag_expr = tag_expr.trim().to_string();
        if tag_expr.is_empty() {
            return;
        }
        self.tag_history.retain(|t| t != &tag_expr);
        self.tag_history.insert(0, tag_expr);
        if self.tag_history.len() > TAG_HISTORY_MAX {
            self.tag_history.truncate(TAG_HISTORY_MAX);
        }
        Self::persist_tag_history(&self.tag_history);
    }

    fn tag_history_path() -> std::path::PathBuf {
        let mut p = dirs_fallback();
        p.push("catpane");
        p.push("tag_history.txt");
        p
    }

    fn load_tag_history() -> Vec<String> {
        let path = Self::tag_history_path();
        std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(TAG_HISTORY_MAX)
            .map(|s| s.to_string())
            .collect()
    }

    fn persist_tag_history(history: &[String]) {
        let path = Self::tag_history_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = history.join("\n");
        let _ = std::fs::write(&path, content);
    }

    // --- Session persistence ---

    fn session_path() -> std::path::PathBuf {
        let mut p = dirs_fallback();
        p.push("catpane");
        p.push("session.json");
        p
    }

    pub fn save_session(&self) {
        let pane_ids = self.pane_tree.pane_ids();
        let id_to_idx: HashMap<PaneId, usize> = pane_ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();

        let session_panes: Vec<SessionPane> = pane_ids
            .iter()
            .filter_map(|id| {
                let pane = self.panes.get(id)?;
                Some(SessionPane {
                    device: pane.device.clone(),
                    package: pane.filter.package.clone(),
                    ios_process: pane.ios_process_filter_text.clone(),
                    ios_subsystem: pane.ios_subsystem_filter_text.clone(),
                    ios_category: pane.ios_category_filter_text.clone(),
                    tag_input: pane.tag_input.clone(),
                    min_level: pane.filter.min_level,
                    hide_vendor_noise: pane.filter.hide_vendor_noise,
                    word_wrap: pane.word_wrap,
                })
            })
            .collect();

        let tree = Self::tree_to_session(&self.pane_tree, &id_to_idx);
        let focused_idx = id_to_idx.get(&self.focused_pane).copied().unwrap_or(0);

        let session = Session {
            panes: session_panes,
            tree,
            focused: focused_idx,
            sidebar_open: self.sidebar_open,
        };

        let path = Self::session_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn try_restore(rt: &tokio::runtime::Handle, devices: &[ConnectedDevice]) -> Option<Self> {
        let path = Self::session_path();
        let json = std::fs::read_to_string(&path).ok()?;
        let session: Session = serde_json::from_str(&json).ok()?;

        if session.panes.is_empty() {
            return None;
        }

        // Create panes from session data
        let mut pane_ids: Vec<PaneId> = Vec::new();
        let mut panes = HashMap::new();

        for sp in &session.panes {
            // If saved device isn't available, fall back to first available
            let device = sp
                .device
                .as_ref()
                .filter(|id| devices.iter().any(|d| &d.id == *id))
                .cloned()
                .or_else(|| devices.first().map(|d| d.id.clone()));

            let mut pane = Pane::new(device);
            pane.filter.min_level = sp.min_level;
            pane.filter.hide_vendor_noise = sp.hide_vendor_noise;
            pane.filter.package = sp.package.clone();
            pane.ios_process_filter_text = sp.ios_process.clone();
            pane.ios_subsystem_filter_text = sp.ios_subsystem.clone();
            pane.ios_category_filter_text = sp.ios_category.clone();
            pane.tag_input = sp.tag_input.clone();
            pane.prev_tag_input = sp.tag_input.clone();
            pane.package_filter_text = sp.package.clone().unwrap_or_default();
            pane.word_wrap = sp.word_wrap;
            pane.apply_tag_filter();
            pane.apply_ios_filters();
            if sp.package.is_some() {
                pane.package_refresh_pending = true;
            }
            pane_ids.push(pane.id);
            panes.insert(pane.id, pane);
        }

        let tree = Self::session_to_tree(&session.tree, &pane_ids)?;
        let focused = pane_ids
            .get(session.focused)
            .copied()
            .unwrap_or(pane_ids[0]);

        let device_tracker = Some(capture::spawn_device_tracker(rt));

        let mut app = Self {
            pane_tree: tree,
            panes,
            focused_pane: focused,
            sidebar_open: session.sidebar_open,
            devices: devices.to_vec(),
            rt: rt.clone(),
            show_help: false,
            device_refresh_pending: false,
            ios_simulator_refresh_pending: false,
            tag_history: Self::load_tag_history(),
            shared_captures: HashMap::new(),
            device_tracker,
            wireless_pair_host: catpane_core::adb::local_ip_prefix(),
            wireless_pair_code: String::new(),
            wireless_connect_host: catpane_core::adb::local_ip_prefix(),
            wireless_status: None,
            wireless_usb_device: None,
            qr_pairing: None,
            ios_simulators: Vec::new(),
            ios_simulator_status: None,
            ios_simulator_booting_udid: None,
            ios_simulator_boot_rx: None,
            location_lat: String::new(),
            location_lon: String::new(),
            location_preset: "Custom".to_string(),
            location_status: None,
            location_rx: None,
        };

        for pane_id in pane_ids {
            app.ensure_pane_capture(pane_id);
        }

        Some(app)
    }

    fn tree_to_session(node: &PaneNode, id_map: &HashMap<PaneId, usize>) -> SessionTree {
        match node {
            PaneNode::Leaf(id) => SessionTree::Leaf(*id_map.get(id).unwrap_or(&0)),
            PaneNode::Split { dir, first, second } => SessionTree::Split {
                dir: match dir {
                    SplitDir::Horizontal => "h".to_string(),
                    SplitDir::Vertical => "v".to_string(),
                },
                first: Box::new(Self::tree_to_session(first, id_map)),
                second: Box::new(Self::tree_to_session(second, id_map)),
            },
        }
    }

    fn session_to_tree(node: &SessionTree, pane_ids: &[PaneId]) -> Option<PaneNode> {
        match node {
            SessionTree::Leaf(idx) => {
                let id = *pane_ids.get(*idx)?;
                Some(PaneNode::Leaf(id))
            }
            SessionTree::Split { dir, first, second } => {
                let split_dir = if dir == "h" {
                    SplitDir::Horizontal
                } else {
                    SplitDir::Vertical
                };
                Some(PaneNode::Split {
                    dir: split_dir,
                    first: Box::new(Self::session_to_tree(first, pane_ids)?),
                    second: Box::new(Self::session_to_tree(second, pane_ids)?),
                })
            }
        }
    }
}

fn dirs_fallback() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let mut p = std::path::PathBuf::from(home);
        p.push(".config");
        p
    } else if let Ok(appdata) = std::env::var("APPDATA") {
        std::path::PathBuf::from(appdata)
    } else {
        std::path::PathBuf::from(".")
    }
}

#[cfg(target_os = "macos")]
fn macos_app_bundle_for_executable(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }

    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()? != "Contents" {
        return None;
    }

    let app_dir = contents_dir.parent()?;
    if app_dir.extension()? != std::ffi::OsStr::new("app") {
        return None;
    }

    Some(app_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::SessionPane;
    use crate::pane::Pane;
    use catpane_core::log_entry::LogLevel;

    #[test]
    fn pane_defaults_to_word_wrap_enabled() {
        assert!(Pane::new(None).word_wrap);
    }

    #[test]
    fn session_pane_missing_word_wrap_defaults_to_enabled() {
        let pane: SessionPane = serde_json::from_str(
            r#"{
                "device": null,
                "package": null,
                "tag_input": "",
                "min_level": "Verbose",
                "hide_vendor_noise": false
            }"#,
        )
        .unwrap();

        assert!(pane.word_wrap);
    }

    #[test]
    fn session_pane_preserves_explicit_word_wrap_value() {
        let pane: SessionPane = serde_json::from_str(
            r#"{
                "device": null,
                "package": null,
                "tag_input": "",
                "min_level": "Verbose",
                "hide_vendor_noise": false,
                "word_wrap": false
            }"#,
        )
        .unwrap();

        assert_eq!(pane.min_level, LogLevel::Verbose);
        assert!(!pane.word_wrap);
    }

    #[test]
    fn session_missing_sidebar_open_defaults_to_collapsed() {
        let session: super::Session = serde_json::from_str(
            r#"{
                "panes": [{
                    "device": null,
                    "package": null,
                    "tag_input": "",
                    "min_level": "Verbose",
                    "hide_vendor_noise": false
                }],
                "tree": {"Leaf": 0},
                "focused": 0
            }"#,
        )
        .unwrap();

        assert!(!session.sidebar_open);
    }
}
