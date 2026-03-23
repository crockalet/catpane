use crate::adb::{self, LogcatHandle};
use crate::filter::Filter;
use crate::log_entry::{LogEntry, parse_logcat_line};

const MAX_LOG_ENTRIES: usize = 50_000;

pub type PaneId = u64;

static NEXT_PANE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_pane_id() -> PaneId {
    NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

pub enum PaneNode {
    Leaf(PaneId),
    Split {
        dir: SplitDir,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    pub fn leaf(id: PaneId) -> Self {
        PaneNode::Leaf(id)
    }

    pub fn split(&mut self, target: PaneId, dir: SplitDir, new_id: PaneId) -> bool {
        match self {
            PaneNode::Leaf(id) if *id == target => {
                let old = PaneNode::Leaf(*id);
                let new = PaneNode::Leaf(new_id);
                *self = PaneNode::Split {
                    dir,
                    first: Box::new(old),
                    second: Box::new(new),
                };
                true
            }
            PaneNode::Split { first, second, .. } => {
                first.split(target, dir, new_id) || second.split(target, dir, new_id)
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, target: PaneId) -> bool {
        match self {
            PaneNode::Split { first, second, .. } => {
                if let PaneNode::Leaf(id) = first.as_ref() {
                    if *id == target {
                        *self = std::mem::replace(second.as_mut(), PaneNode::Leaf(0));
                        return true;
                    }
                }
                if let PaneNode::Leaf(id) = second.as_ref() {
                    if *id == target {
                        *self = std::mem::replace(first.as_mut(), PaneNode::Leaf(0));
                        return true;
                    }
                }
                first.remove(target) || second.remove(target)
            }
            _ => false,
        }
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        match self {
            PaneNode::Leaf(id) => vec![*id],
            PaneNode::Split { first, second, .. } => {
                let mut ids = first.pane_ids();
                ids.extend(second.pane_ids());
                ids
            }
        }
    }

    pub fn count(&self) -> usize {
        match self {
            PaneNode::Leaf(_) => 1,
            PaneNode::Split { first, second, .. } => first.count() + second.count(),
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            PaneNode::Leaf(_) => 0,
            PaneNode::Split { first, second, .. } => 1 + first.depth().max(second.depth()),
        }
    }

    /// Returns the depth of the node containing the target pane.
    pub fn depth_of(&self, target: PaneId) -> Option<usize> {
        match self {
            PaneNode::Leaf(id) => {
                if *id == target {
                    Some(0)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => first
                .depth_of(target)
                .map(|d| d + 1)
                .or_else(|| second.depth_of(target).map(|d| d + 1)),
        }
    }
}

pub struct Pane {
    pub id: PaneId,
    pub device: Option<String>,
    pub filter: Filter,
    pub entries: Vec<LogEntry>,
    pub filtered_indices: Vec<usize>,
    pub scroll_to_bottom: bool,
    pub auto_scroll: bool,
    pub paused: bool,
    pub logcat_handle: Option<LogcatHandle>,
    pub pid_filter: Option<u32>,
    // Search
    pub search_open: bool,
    pub search_input: String,
    pub search_match_indices: Vec<usize>,
    pub search_current: usize,
    // Tag filter
    pub tag_input: String,
    pub prev_tag_input: String,
    // Package filter text in popup
    pub package_filter_text: String,
    pub pkg_selection_index: i32,
    // Tag suggestion keyboard selection index
    pub tag_suggestion_index: i32,
    // All unique tags seen in this session
    pub seen_tags: Vec<String>,
    // Scroll to a specific filtered index (set by search navigation)
    pub scroll_to_fi: Option<usize>,
    // Request focus on the search text input (set on Cmd+F)
    pub search_focus_requested: bool,
    // Row selection (filtered indices) — supports range via shift-click
    pub selection_anchor: Option<usize>,
    pub selection_end: Option<usize>,
    // Packages for this pane's device
    pub packages: Vec<String>,
    pub package_refresh_pending: bool,
    // Timestamp of the last pid-only re-poll (to detect package restarts)
    pub last_pid_poll: std::time::Instant,
    // Display options
    pub word_wrap: bool,
    // Scroll offset managed by us (not egui's id-based state) to survive
    // pane tree restructures that change the ScrollArea's internal id.
    pub scroll_offset_y: f32,
    // Cooldown for auto-restart of dead logcat (avoid rapid-fire respawn loops)
    pub last_logcat_restart: std::time::Instant,
}

impl Pane {
    pub fn selected_range(&self) -> Option<(usize, usize)> {
        match (self.selection_anchor, self.selection_end) {
            (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
            (Some(a), None) => Some((a, a)),
            _ => None,
        }
    }

    pub fn is_row_selected(&self, fi: usize) -> bool {
        self.selected_range()
            .is_some_and(|(lo, hi)| fi >= lo && fi <= hi)
    }

    pub fn new(device: Option<String>) -> Self {
        Self {
            id: next_pane_id(),
            device,
            filter: Filter::default(),
            entries: Vec::with_capacity(1024),
            filtered_indices: Vec::with_capacity(1024),
            scroll_to_bottom: true,
            auto_scroll: true,
            paused: false,
            logcat_handle: None,
            pid_filter: None,
            search_open: false,
            search_input: String::new(),
            search_match_indices: Vec::new(),
            search_current: 0,
            tag_input: String::new(),
            prev_tag_input: String::new(),
            package_filter_text: String::new(),
            pkg_selection_index: -1,
            tag_suggestion_index: -1,
            seen_tags: Vec::new(),
            scroll_to_fi: None,
            search_focus_requested: false,
            selection_anchor: None,
            selection_end: None,
            packages: Vec::new(),
            package_refresh_pending: false,
            last_pid_poll: std::time::Instant::now(),
            word_wrap: false,
            scroll_offset_y: 0.0,
            last_logcat_restart: std::time::Instant::now(),
        }
    }

    pub fn ingest_lines(&mut self) {
        if self.paused {
            if let Some(ref mut handle) = self.logcat_handle {
                while handle.rx.try_recv().is_ok() {}
            }
            return;
        }

        let mut added = false;
        let mut channel_dead = false;
        if let Some(ref mut handle) = self.logcat_handle {
            for _ in 0..500 {
                match handle.rx.try_recv() {
                    Ok(line) => {
                        if let Some(entry) = parse_logcat_line(&line) {
                            // Track unique tags
                            if !self.seen_tags.contains(&entry.tag) {
                                self.seen_tags.push(entry.tag.clone());
                            }
                            let idx = self.entries.len();
                            let passes = self.filter.matches(&entry, self.pid_filter);
                            self.entries.push(entry);
                            if passes {
                                self.filtered_indices.push(idx);
                                added = true;
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        channel_dead = true;
                        break;
                    }
                }
            }
        }

        // Logcat process exited — drop the dead handle so the update loop
        // can detect this and restart when the device is available.
        if channel_dead {
            self.logcat_handle = None;
        }

        if added && self.auto_scroll {
            self.scroll_to_bottom = true;
        }

        if self.entries.len() > MAX_LOG_ENTRIES {
            self.compact();
        }
    }

    fn compact(&mut self) {
        let drain_count = MAX_LOG_ENTRIES / 4;
        self.entries.drain(0..drain_count);
        self.rebuild_filtered();
    }

    pub fn rebuild_filtered(&mut self) {
        self.filtered_indices.clear();
        self.search_match_indices.clear();

        for (i, entry) in self.entries.iter().enumerate() {
            if self.filter.matches(entry, self.pid_filter) {
                self.filtered_indices.push(i);
                if self.search_open && self.filter.matches_search(entry) {
                    self.search_match_indices
                        .push(self.filtered_indices.len() - 1);
                }
            }
        }

        self.scroll_to_bottom = true;
    }

    pub fn update_search(&mut self) {
        self.filter.set_search(&self.search_input.clone());
        self.search_match_indices.clear();
        if self.search_open && self.filter.search_regex.is_some() {
            for (fi, &ei) in self.filtered_indices.iter().enumerate() {
                if self.filter.matches_search(&self.entries[ei]) {
                    self.search_match_indices.push(fi);
                }
            }
        }
        self.search_current = 0;
    }

    pub fn search_next(&mut self) -> Option<usize> {
        if self.search_match_indices.is_empty() {
            return None;
        }
        self.search_current = (self.search_current + 1) % self.search_match_indices.len();
        Some(self.search_match_indices[self.search_current])
    }

    pub fn search_prev(&mut self) -> Option<usize> {
        if self.search_match_indices.is_empty() {
            return None;
        }
        if self.search_current == 0 {
            self.search_current = self.search_match_indices.len() - 1;
        } else {
            self.search_current -= 1;
        }
        Some(self.search_match_indices[self.search_current])
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.filtered_indices.clear();
        self.search_match_indices.clear();
    }

    pub fn apply_tag_filter(&mut self) {
        self.filter.tag_filters = Filter::parse_tag_filters(&self.tag_input.clone());
        self.rebuild_filtered();
    }

    pub fn start_logcat(&mut self, rt: &tokio::runtime::Handle) {
        self.stop_logcat();
        if let Some(ref device) = self.device {
            self.logcat_handle = Some(adb::spawn_logcat(rt, device.clone(), self.pid_filter));
            self.last_logcat_restart = std::time::Instant::now();
        }
    }

    pub fn stop_logcat(&mut self) {
        if let Some(ref handle) = self.logcat_handle {
            handle.stop();
        }
        self.logcat_handle = None;
    }
}
