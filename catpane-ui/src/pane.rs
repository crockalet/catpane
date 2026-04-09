use std::collections::{HashMap, HashSet};

use catpane_core::crash_detector::{CrashDetector, CrashReport, CrashType, detect_crashes};
use catpane_core::filter::Filter;
use catpane_core::log_buffer_config::log_buffer_capacity;
use catpane_core::log_entry::LogEntry;
use tokio::sync::broadcast;

const COMPACT_BATCH_DIVISOR: usize = 50;
const MIN_COMPACT_BATCH_SIZE: usize = 250;

pub const WATCH_COLORS: &[(u8, u8, u8)] = &[
    (0, 180, 216),   // cyan
    (255, 183, 77),  // orange
    (129, 199, 132), // green
    (186, 104, 200), // purple
    (255, 138, 128), // coral
    (100, 181, 246), // blue
];

#[derive(Clone)]
pub struct UiWatch {
    pub name: String,
    pub pattern: String,
    pub pattern_lower: String,
    pub color_index: usize,
    pub match_count: usize,
}

impl UiWatch {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        let pat = &self.pattern_lower;
        let msg = entry.message.to_ascii_lowercase();
        if msg.contains(pat.as_str()) {
            return true;
        }
        if entry.tag.to_ascii_lowercase().contains(pat.as_str()) {
            return true;
        }
        false
    }
}

const SAVED_CRASH_CAP: usize = 100;
const CRASH_CONTEXT_BEFORE: usize = 10;
const CRASH_CONTEXT_AFTER: usize = 5;

#[derive(Clone)]
pub struct SavedCrash {
    pub crash_type: CrashType,
    pub summary: String,
    pub timestamp: String,
    pub pid: u32,
    pub context_lines: Vec<LogEntry>,
    pub crash_start_offset: usize,
    pub crash_end_offset: usize,
}

pub type PaneId = u64;

static NEXT_PANE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_pane_id() -> PaneId {
    NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn default_word_wrap() -> bool {
    true
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
    pub capture_rx: Option<broadcast::Receiver<LogEntry>>,
    pub capture_device_id: Option<String>,
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
    pub ios_process_filter_text: String,
    pub ios_subsystem_filter_text: String,
    pub ios_category_filter_text: String,
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
    pub last_capture_restart: std::time::Instant,
    /// Indices into `entries` that are part of a crash (for highlighting)
    pub crash_line_indices: HashSet<usize>,
    /// Crash reports detected in current entries
    pub crash_reports: Vec<CrashReport>,
    /// Current crash navigation index (into crash_reports)
    pub crash_nav_index: Option<usize>,
    /// Active log pattern watches
    pub watches: Vec<UiWatch>,
    /// Text field for new watch pattern input
    pub watch_input: String,
    /// entry_index -> color_index for highlighted watch matches
    pub watch_highlights: HashMap<usize, usize>,
    /// Filtered index count at last incremental watch scan
    last_watch_scanned_count: usize,
    /// Incremental crash detector (avoids full rescan each frame)
    crash_detector: CrashDetector,
    /// Crash logs with surrounding context that survive clear()
    pub saved_crashes: Vec<SavedCrash>,
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
            capture_rx: None,
            capture_device_id: None,
            pid_filter: None,
            search_open: false,
            search_input: String::new(),
            search_match_indices: Vec::new(),
            search_current: 0,
            tag_input: String::new(),
            prev_tag_input: String::new(),
            package_filter_text: String::new(),
            ios_process_filter_text: String::new(),
            ios_subsystem_filter_text: String::new(),
            ios_category_filter_text: String::new(),
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
            word_wrap: default_word_wrap(),
            scroll_offset_y: 0.0,
            last_capture_restart: std::time::Instant::now(),
            crash_line_indices: HashSet::new(),
            crash_reports: Vec::new(),
            crash_nav_index: None,
            watches: Vec::new(),
            watch_input: String::new(),
            watch_highlights: HashMap::new(),
            last_watch_scanned_count: 0,
            crash_detector: CrashDetector::new(),
            saved_crashes: Vec::new(),
        }
    }

    pub fn ingest_lines(&mut self) {
        if self.paused {
            if let Some(ref mut rx) = self.capture_rx {
                while rx.try_recv().is_ok() {}
            }
            return;
        }

        let mut added = false;
        let mut channel_dead = false;
        let mut new_crash_reports: Vec<CrashReport> = Vec::new();
        if let Some(ref mut rx) = self.capture_rx {
            for _ in 0..500 {
                match rx.try_recv() {
                    Ok(entry) => {
                        if !self.seen_tags.contains(&entry.tag) {
                            self.seen_tags.push(entry.tag.clone());
                        }
                        let idx = self.entries.len();
                        let passes = self.filter.matches(&entry, self.pid_filter);
                        // Feed to crash detector before pushing (avoids borrow conflict)
                        if let Some(report) = self.crash_detector.feed(idx, &entry) {
                            new_crash_reports.push(report);
                        }
                        self.entries.push(entry);
                        if passes {
                            self.filtered_indices.push(idx);
                            added = true;
                        }
                    }
                    Err(broadcast::error::TryRecvError::Empty) => break,
                    Err(broadcast::error::TryRecvError::Closed) => {
                        channel_dead = true;
                        break;
                    }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                }
            }
        }

        // Flush any pending partial crash at end of batch
        if let Some(report) = self.crash_detector.flush() {
            new_crash_reports.push(report);
        }

        // Capture process exited — drop the dead handle so the update loop
        // can detect this and restart when the device is available.
        if channel_dead {
            self.capture_rx = None;
        }

        if added && self.auto_scroll {
            self.scroll_to_bottom = true;
        }

        // Incrementally append new crash reports and update indices
        if !new_crash_reports.is_empty() {
            for report in &new_crash_reports {
                for i in report.first_index..=report.last_index {
                    self.crash_line_indices.insert(i);
                }
            }
            // Save crashes with surrounding context
            self.save_crashes(&new_crash_reports);
            self.crash_reports.extend(new_crash_reports);
        }

        // Incrementally update watch highlights for newly ingested entries
        if added && !self.watches.is_empty() {
            let scan_start = self.last_watch_scanned_count;
            for &entry_idx in &self.filtered_indices[scan_start..] {
                if self.watch_highlights.contains_key(&entry_idx) {
                    continue;
                }
                if let Some(entry) = self.entries.get(entry_idx) {
                    for watch in self.watches.iter_mut() {
                        if watch.matches(entry) {
                            watch.match_count += 1;
                            self.watch_highlights.insert(entry_idx, watch.color_index);
                            break;
                        }
                    }
                }
            }
        }
        if added {
            self.last_watch_scanned_count = self.filtered_indices.len();
        }

        let capacity = log_buffer_capacity();
        if self.entries.len() > compact_threshold(capacity) {
            self.compact();
        }
    }

    fn compact(&mut self) {
        let capacity = log_buffer_capacity();
        let drain_count = self.entries.len().saturating_sub(capacity);
        if drain_count == 0 {
            return;
        }
        let scroll_to_bottom = self.scroll_to_bottom;
        self.entries.drain(0..drain_count);
        self.rebuild_filtered();
        // Full rebuild of crash state after compaction shifts indices
        self.crash_detector = CrashDetector::new();
        self.crash_reports = detect_crashes(&self.entries);
        self.crash_line_indices.clear();
        for report in &self.crash_reports {
            for i in report.first_index..=report.last_index {
                self.crash_line_indices.insert(i);
            }
        }
        self.crash_nav_index = None;
        self.scroll_to_bottom = scroll_to_bottom;
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

        self.rebuild_watch_highlights();
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
        self.crash_line_indices.clear();
        self.crash_reports.clear();
        self.crash_nav_index = None;
        self.crash_detector = CrashDetector::new();
        self.watch_highlights.clear();
        self.last_watch_scanned_count = 0;
        for watch in &mut self.watches {
            watch.match_count = 0;
        }
        // Note: saved_crashes intentionally NOT cleared
    }

    pub fn apply_tag_filter(&mut self) {
        self.filter.tag_filters = Filter::parse_tag_filters(&self.tag_input.clone());
        self.rebuild_filtered();
    }

    pub fn apply_ios_filters(&mut self) {
        self.filter.ios_process = normalize_optional_string(&self.ios_process_filter_text);
        self.filter.ios_subsystem = normalize_optional_string(&self.ios_subsystem_filter_text);
        self.filter.ios_category = normalize_optional_string(&self.ios_category_filter_text);
        self.rebuild_filtered();
    }

    pub fn start_capture(&mut self, device_id: String, capture_rx: broadcast::Receiver<LogEntry>) {
        self.stop_capture();
        self.capture_device_id = Some(device_id);
        self.capture_rx = Some(capture_rx);
        self.last_capture_restart = std::time::Instant::now();
    }

    pub fn stop_capture(&mut self) {
        self.capture_rx = None;
        self.capture_device_id = None;
    }

    /// Re-run crash detection on current entries (full rebuild).
    pub fn rebuild_crashes(&mut self) {
        self.crash_detector = CrashDetector::new();
        self.crash_reports = detect_crashes(&self.entries);
        self.crash_line_indices.clear();
        for report in &self.crash_reports {
            for i in report.first_index..=report.last_index {
                self.crash_line_indices.insert(i);
            }
        }
        self.crash_nav_index = None;
    }

    /// Navigate to the next crash. Returns the filtered_index to scroll to, if any.
    pub fn next_crash(&mut self) -> Option<usize> {
        if self.crash_reports.is_empty() {
            return None;
        }
        let idx = match self.crash_nav_index {
            Some(i) => (i + 1) % self.crash_reports.len(),
            None => 0,
        };
        self.crash_nav_index = Some(idx);
        let entry_idx = self.crash_reports[idx].first_index;
        self.filtered_indices.iter().position(|&fi| fi >= entry_idx)
    }

    /// Navigate to the previous crash.
    pub fn prev_crash(&mut self) -> Option<usize> {
        if self.crash_reports.is_empty() {
            return None;
        }
        let idx = match self.crash_nav_index {
            Some(0) | None => self.crash_reports.len() - 1,
            Some(i) => i - 1,
        };
        self.crash_nav_index = Some(idx);
        let entry_idx = self.crash_reports[idx].first_index;
        self.filtered_indices.iter().position(|&fi| fi >= entry_idx)
    }

    pub fn add_watch(&mut self, name: String, pattern: String) {
        let color_index = self.watches.len() % WATCH_COLORS.len();
        let pattern_lower = pattern.to_ascii_lowercase();
        self.watches.push(UiWatch {
            name,
            pattern,
            pattern_lower,
            color_index,
            match_count: 0,
        });
        self.rebuild_watch_highlights();
    }

    pub fn remove_watch(&mut self, index: usize) {
        if index < self.watches.len() {
            self.watches.remove(index);
            self.rebuild_watch_highlights();
        }
    }

    pub fn rebuild_watch_highlights(&mut self) {
        self.watch_highlights.clear();
        for watch in self.watches.iter_mut() {
            watch.match_count = 0;
        }
        for &entry_idx in &self.filtered_indices {
            if let Some(entry) = self.entries.get(entry_idx) {
                for watch in self.watches.iter_mut() {
                    if watch.matches(entry) {
                        watch.match_count += 1;
                        self.watch_highlights.insert(entry_idx, watch.color_index);
                        break; // first matching watch wins for color
                    }
                }
            }
        }
        self.last_watch_scanned_count = self.filtered_indices.len();
    }

    fn save_crashes(&mut self, reports: &[CrashReport]) {
        for report in reports {
            let first = report.first_index;
            let last = report.last_index;
            let ctx_start = first.saturating_sub(CRASH_CONTEXT_BEFORE);
            let ctx_end = (last + CRASH_CONTEXT_AFTER + 1).min(self.entries.len());
            let context_lines: Vec<LogEntry> = self.entries[ctx_start..ctx_end]
                .iter()
                .cloned()
                .collect();
            let crash_start_offset = first - ctx_start;
            let crash_end_offset = last - ctx_start;

            self.saved_crashes.push(SavedCrash {
                crash_type: report.crash_type,
                summary: report.headline.clone(),
                timestamp: report.timestamp.clone(),
                pid: report.pid.unwrap_or(0),
                context_lines,
                crash_start_offset,
                crash_end_offset,
            });
        }

        // Cap at SAVED_CRASH_CAP, dropping oldest
        if self.saved_crashes.len() > SAVED_CRASH_CAP {
            let excess = self.saved_crashes.len() - SAVED_CRASH_CAP;
            self.saved_crashes.drain(0..excess);
        }
    }
}

fn normalize_optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn compact_threshold(capacity: usize) -> usize {
    capacity.saturating_add(compact_batch_size(capacity))
}

fn compact_batch_size(capacity: usize) -> usize {
    (capacity / COMPACT_BATCH_DIVISOR)
        .max(MIN_COMPACT_BATCH_SIZE)
        .min(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catpane_core::log_entry::{LogLevel, LogPlatform};

    fn entry(tag: &str, message: &str) -> LogEntry {
        LogEntry {
            platform: LogPlatform::Android,
            timestamp: "03-10 06:30:45.000".to_string(),
            pid: Some(123),
            tid: Some(456),
            level: LogLevel::Info,
            tag: tag.to_string(),
            process: None,
            subsystem: None,
            category: None,
            message: message.to_string(),
        }
    }

    #[test]
    fn compact_keeps_latest_entries_with_consistent_indices() {
        let mut pane = Pane::new(None);
        let capacity = log_buffer_capacity();
        let total = capacity + 3;

        for i in 0..total {
            pane.entries.push(entry("Tag", &format!("message-{i}")));
        }

        pane.rebuild_filtered();
        pane.compact();

        assert_eq!(pane.entries.len(), capacity);
        assert_eq!(pane.filtered_indices.len(), capacity);
        assert_eq!(
            pane.entries.first().map(|entry| entry.message.as_str()),
            Some("message-3")
        );
        assert_eq!(
            pane.entries.last().map(|entry| entry.message.as_str()),
            Some(format!("message-{}", total - 1).as_str())
        );
        assert_eq!(pane.filtered_indices.first().copied(), Some(0));
        assert_eq!(pane.filtered_indices.last().copied(), Some(capacity - 1));
    }

    #[test]
    fn compact_uses_headroom_before_rebuilding() {
        let capacity = log_buffer_capacity();
        let batch_size = compact_batch_size(capacity);
        assert!(compact_threshold(capacity) > capacity);
        assert_eq!(compact_threshold(capacity), capacity + batch_size);
    }

    #[test]
    fn compact_preserves_existing_scroll_intent() {
        let mut pane = Pane::new(None);
        let capacity = log_buffer_capacity();

        for i in 0..=capacity {
            pane.entries.push(entry("Tag", &format!("message-{i}")));
        }

        pane.rebuild_filtered();
        pane.scroll_to_bottom = false;
        pane.compact();

        assert!(!pane.scroll_to_bottom);
    }
}
