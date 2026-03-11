use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::adb::AdbDevice;
use crate::log_entry::LogLevel;
use crate::pane::{Pane, PaneId, PaneNode, SplitDir};

use egui;

const TAG_HISTORY_MAX: usize = 50;

pub struct QrPairingState {
    pub password: String,
    pub service_name: String,
    pub qr_texture: Option<egui::TextureHandle>,
    pub mdns_rx: tokio::sync::mpsc::Receiver<Result<String, String>>,
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
    tag_input: String,
    min_level: LogLevel,
    hide_vendor_noise: bool,
}

#[derive(Serialize, Deserialize)]
enum SessionTree {
    Leaf(usize), // index into panes vec
    Split { dir: String, first: Box<SessionTree>, second: Box<SessionTree> },
}

#[derive(Serialize, Deserialize)]
struct Session {
    panes: Vec<SessionPane>,
    tree: SessionTree,
    focused: usize,
}

pub struct App {
    pub pane_tree: PaneNode,
    pub panes: HashMap<PaneId, Pane>,
    pub focused_pane: PaneId,
    pub devices: Vec<AdbDevice>,
    pub rt: tokio::runtime::Handle,
    pub show_help: bool,
    pub device_refresh_pending: bool,
    pub tag_history: Vec<String>,
    pub device_tracker: Option<tokio::sync::mpsc::Receiver<Vec<AdbDevice>>>,
    // Wireless debugging dialog state
    pub show_wireless_dialog: bool,
    pub wireless_pair_host: String,
    pub wireless_pair_code: String,
    pub wireless_connect_host: String,
    pub wireless_status: Option<(bool, String)>,
    // QR pairing state
    pub qr_pairing: Option<QrPairingState>,
}

impl App {
    pub fn new(rt: tokio::runtime::Handle, devices: Vec<AdbDevice>) -> Self {
        // Try restoring session
        if let Some(app) = Self::try_restore(&rt, &devices) {
            return app;
        }

        let default_device = devices.first().map(|d| d.serial.clone());
        let mut pane = Pane::new(default_device);
        let id = pane.id;
        pane.start_logcat(&rt);

        let mut panes = HashMap::new();
        panes.insert(id, pane);

        let device_tracker = Some(crate::adb::spawn_device_tracker(&rt));

        Self {
            pane_tree: PaneNode::leaf(id),
            panes,
            focused_pane: id,
            devices,
            rt,
            show_help: false,
            device_refresh_pending: false,
            tag_history: Self::load_tag_history(),
            device_tracker,
            show_wireless_dialog: false,
            wireless_pair_host: crate::adb::local_ip_prefix(),
            wireless_pair_code: String::new(),
            wireless_connect_host: crate::adb::local_ip_prefix(),
            wireless_status: None,
            qr_pairing: None,
        }
    }

    pub fn poll_all(&mut self) {
        let ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for id in ids {
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.ingest_lines();
            }
        }
    }

    pub fn split_pane(&mut self, dir: SplitDir) {
        // Limit split depth to 2 (max 4 panes)
        let current_depth = self.pane_tree.depth_of(self.focused_pane).unwrap_or(0);
        if current_depth >= 2 {
            return;
        }

        let device = self.panes.get(&self.focused_pane)
            .and_then(|p| p.device.clone())
            .or_else(|| self.devices.first().map(|d| d.serial.clone()));

        let mut new_pane = Pane::new(device);
        let new_id = new_pane.id;
        new_pane.start_logcat(&self.rt);

        self.pane_tree.split(self.focused_pane, dir, new_id);
        self.panes.insert(new_id, new_pane);
        self.focused_pane = new_id;
    }

    pub fn close_pane(&mut self, id: PaneId) {
        if self.pane_tree.count() <= 1 { return; }
        if let Some(mut pane) = self.panes.remove(&id) {
            pane.stop_logcat();
        }
        self.pane_tree.remove(id);
        let ids = self.pane_tree.pane_ids();
        if !ids.contains(&self.focused_pane) {
            self.focused_pane = ids.first().copied().unwrap_or(0);
        }
    }

    pub fn cycle_focus(&mut self) {
        let ids = self.pane_tree.pane_ids();
        if ids.is_empty() { return; }
        let pos = ids.iter().position(|&id| id == self.focused_pane).unwrap_or(0);
        self.focused_pane = ids[(pos + 1) % ids.len()];
    }

    pub fn spawn_new_window(&self) {
        let exe = std::env::current_exe().unwrap_or_default();
        let exe_str = exe.to_string_lossy().to_string();

        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .args(["-na", &exe_str])
                .spawn();
        }

        #[cfg(target_os = "linux")]
        {
            for term in &["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
                if std::process::Command::new(term).args(["-e", &exe_str]).spawn().is_ok() {
                    break;
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/c", "start", &exe_str])
                .spawn();
        }
    }

    pub fn save_tag_to_history(&mut self, tag_expr: &str) {
        let tag_expr = tag_expr.trim().to_string();
        if tag_expr.is_empty() { return; }
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
        let id_to_idx: HashMap<PaneId, usize> = pane_ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let session_panes: Vec<SessionPane> = pane_ids.iter().filter_map(|id| {
            let pane = self.panes.get(id)?;
            Some(SessionPane {
                device: pane.device.clone(),
                package: pane.filter.package.clone(),
                tag_input: pane.tag_input.clone(),
                min_level: pane.filter.min_level,
                hide_vendor_noise: pane.filter.hide_vendor_noise,
            })
        }).collect();

        let tree = Self::tree_to_session(&self.pane_tree, &id_to_idx);
        let focused_idx = id_to_idx.get(&self.focused_pane).copied().unwrap_or(0);

        let session = Session {
            panes: session_panes,
            tree,
            focused: focused_idx,
        };

        let path = Self::session_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn try_restore(rt: &tokio::runtime::Handle, devices: &[AdbDevice]) -> Option<Self> {
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
            let device = sp.device.as_ref()
                .filter(|serial| devices.iter().any(|d| &d.serial == *serial))
                .cloned()
                .or_else(|| devices.first().map(|d| d.serial.clone()));

            let mut pane = Pane::new(device);
            pane.filter.min_level = sp.min_level;
            pane.filter.hide_vendor_noise = sp.hide_vendor_noise;
            pane.filter.package = sp.package.clone();
            pane.tag_input = sp.tag_input.clone();
            pane.prev_tag_input = sp.tag_input.clone();
            pane.package_filter_text = sp.package.clone().unwrap_or_default();
            pane.apply_tag_filter();
            pane.start_logcat(rt);

            pane_ids.push(pane.id);
            panes.insert(pane.id, pane);
        }

        let tree = Self::session_to_tree(&session.tree, &pane_ids)?;
        let focused = pane_ids.get(session.focused).copied().unwrap_or(pane_ids[0]);

        let device_tracker = Some(crate::adb::spawn_device_tracker(rt));

        Some(Self {
            pane_tree: tree,
            panes,
            focused_pane: focused,
            devices: devices.to_vec(),
            rt: rt.clone(),
            show_help: false,
            device_refresh_pending: false,
            tag_history: Self::load_tag_history(),
            device_tracker,
            show_wireless_dialog: false,
            wireless_pair_host: crate::adb::local_ip_prefix(),
            wireless_pair_code: String::new(),
            wireless_connect_host: crate::adb::local_ip_prefix(),
            wireless_status: None,
            qr_pairing: None,
        })
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
                let split_dir = if dir == "h" { SplitDir::Horizontal } else { SplitDir::Vertical };
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
