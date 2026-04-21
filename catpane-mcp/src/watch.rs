use catpane_core::log_entry::{LogEntry, LogLevel};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::log_buffer::BufferedLogEntry;

static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);
const DEFAULT_MATCH_RETENTION_CAPACITY: usize = 2_048;

fn next_watch_id() -> String {
    format!("w{}", NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    Text,
    Regex,
}

#[derive(Debug, Clone)]
pub struct Watch {
    pub id: String,
    pub name: String,
    pattern_type: PatternType,
    text_pattern: Option<String>,
    regex_pattern: Option<Regex>,
    tag_filter: Option<String>,
    min_level: Option<LogLevel>,
    pub match_count: u64,
    pub last_match_seq: Option<u64>,
    pub created_at_ms: u64,
    retained_matches: VecDeque<BufferedLogEntry>,
    retained_match_capacity: usize,
    retained_dropped: u64,
}

impl Watch {
    pub fn new_text(
        name: String,
        pattern: String,
        tag: Option<String>,
        min_level: Option<LogLevel>,
    ) -> Self {
        Self::new_text_with_retention(
            name,
            pattern,
            tag,
            min_level,
            DEFAULT_MATCH_RETENTION_CAPACITY,
        )
    }

    pub fn new_text_with_retention(
        name: String,
        pattern: String,
        tag: Option<String>,
        min_level: Option<LogLevel>,
        retained_match_capacity: usize,
    ) -> Self {
        Self {
            id: next_watch_id(),
            name,
            pattern_type: PatternType::Text,
            text_pattern: Some(pattern),
            regex_pattern: None,
            tag_filter: tag,
            min_level,
            match_count: 0,
            last_match_seq: None,
            created_at_ms: now_ms(),
            retained_matches: VecDeque::with_capacity(retained_match_capacity.max(1)),
            retained_match_capacity: retained_match_capacity.max(1),
            retained_dropped: 0,
        }
    }

    pub fn new_regex(
        name: String,
        pattern: &str,
        tag: Option<String>,
        min_level: Option<LogLevel>,
    ) -> Result<Self, String> {
        Self::new_regex_with_retention(
            name,
            pattern,
            tag,
            min_level,
            DEFAULT_MATCH_RETENTION_CAPACITY,
        )
    }

    pub fn new_regex_with_retention(
        name: String,
        pattern: &str,
        tag: Option<String>,
        min_level: Option<LogLevel>,
        retained_match_capacity: usize,
    ) -> Result<Self, String> {
        let re = Regex::new(pattern).map_err(|e| format!("invalid regex: {e}"))?;
        Ok(Self {
            id: next_watch_id(),
            name,
            pattern_type: PatternType::Regex,
            text_pattern: Some(pattern.to_string()),
            regex_pattern: Some(re),
            tag_filter: tag,
            min_level,
            match_count: 0,
            last_match_seq: None,
            created_at_ms: now_ms(),
            retained_matches: VecDeque::with_capacity(retained_match_capacity.max(1)),
            retained_match_capacity: retained_match_capacity.max(1),
            retained_dropped: 0,
        })
    }

    /// Test whether a log entry matches this watch's pattern.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if let Some(min) = self.min_level {
            if entry.level < min {
                return false;
            }
        }

        if let Some(ref tag_f) = self.tag_filter {
            let tag_lower = entry.tag.to_lowercase();
            if !tag_lower.contains(&tag_f.to_lowercase()) {
                return false;
            }
        }

        match self.pattern_type {
            PatternType::Text => {
                let pat = self
                    .text_pattern
                    .as_ref()
                    .expect("text watch must have text_pattern")
                    .to_lowercase();
                let msg_lower = entry.message.to_lowercase();
                let tag_lower = entry.tag.to_lowercase();
                msg_lower.contains(&pat) || tag_lower.contains(&pat)
            }
            PatternType::Regex => {
                let re = self
                    .regex_pattern
                    .as_ref()
                    .expect("regex watch must have regex_pattern");
                re.is_match(&entry.message)
            }
        }
    }

    /// Record a matching buffered entry into the retained side buffer.
    pub fn retain_if_match(&mut self, buffered: &BufferedLogEntry) -> bool {
        if !self.matches(&buffered.entry) {
            return false;
        }

        self.record_match(buffered.clone())
    }

    pub fn retained_matches_since(
        &self,
        since_seq: Option<u64>,
        limit: usize,
    ) -> Vec<BufferedLogEntry> {
        self.retained_matches
            .iter()
            .filter(|buffered| since_seq.is_none_or(|seq| buffered.seq > seq))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn clear_matches(&mut self) -> usize {
        let cleared = self.retained_matches.len();
        self.retained_matches.clear();
        self.match_count = 0;
        self.last_match_seq = None;
        self.retained_dropped = 0;
        cleared
    }

    pub fn summary(&self) -> WatchSummary {
        WatchSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            pattern: self.pattern_display().to_string(),
            pattern_type: self.pattern_type.clone(),
            tag_filter: self.tag_filter.clone(),
            min_level: self.min_level.map(|l| l.label().to_string()),
            match_count: self.match_count,
            last_match_seq: self.last_match_seq,
            retained_count: self.retained_matches.len(),
            retained_capacity: self.retained_match_capacity,
            retained_dropped: self.retained_dropped,
        }
    }

    pub fn pattern_display(&self) -> &str {
        self.text_pattern.as_deref().unwrap_or("")
    }

    fn record_match(&mut self, buffered: BufferedLogEntry) -> bool {
        if self
            .retained_matches
            .iter()
            .any(|existing| existing.seq == buffered.seq)
        {
            return false;
        }

        if self.retained_matches.len() == self.retained_match_capacity {
            self.retained_matches.pop_front();
            self.retained_dropped = self.retained_dropped.saturating_add(1);
        }
        self.match_count = self.match_count.saturating_add(1);
        self.last_match_seq = Some(buffered.seq);
        self.retained_matches.push_back(buffered);
        true
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchSummary {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub pattern_type: PatternType,
    pub tag_filter: Option<String>,
    pub min_level: Option<String>,
    pub match_count: u64,
    pub last_match_seq: Option<u64>,
    pub retained_count: usize,
    pub retained_capacity: usize,
    pub retained_dropped: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WatchRetentionStats {
    pub watch_count: usize,
    pub retained_count: usize,
    pub retained_capacity: usize,
    pub retained_dropped: u64,
}

#[derive(Debug, Default)]
pub struct WatchSet {
    watches: HashMap<String, Watch>,
}

impl WatchSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, watch: Watch) -> String {
        let id = watch.id.clone();
        self.watches.insert(id.clone(), watch);
        id
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.watches.remove(id).is_some()
    }

    pub fn get(&self, id: &str) -> Option<&Watch> {
        self.watches.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Watch> {
        self.watches.get_mut(id)
    }

    pub fn list(&self) -> Vec<WatchSummary> {
        let mut summaries: Vec<_> = self.watches.values().map(|w| w.summary()).collect();
        summaries.sort_by(|a, b| a.id.cmp(&b.id));
        summaries
    }

    pub fn record_entry(&mut self, buffered: &BufferedLogEntry) {
        for watch in self.watches.values_mut() {
            watch.retain_if_match(buffered);
        }
    }

    pub fn seed_matches(&mut self, id: &str, entries: &[BufferedLogEntry]) -> usize {
        let Some(watch) = self.watches.get_mut(id) else {
            return 0;
        };

        let mut seeded = 0;
        for buffered in entries {
            if watch.retain_if_match(buffered) {
                seeded += 1;
            }
        }
        seeded
    }

    pub fn retained_matches(
        &self,
        id: &str,
        since_seq: Option<u64>,
        limit: usize,
    ) -> Option<Vec<BufferedLogEntry>> {
        self.get(id)
            .map(|watch| watch.retained_matches_since(since_seq, limit))
    }

    pub fn clear_matches(&mut self) -> usize {
        self.watches
            .values_mut()
            .map(Watch::clear_matches)
            .sum::<usize>()
    }

    pub fn retention_stats(&self) -> WatchRetentionStats {
        self.watches
            .values()
            .fold(WatchRetentionStats::default(), |mut stats, watch| {
                stats.watch_count += 1;
                stats.retained_count += watch.retained_matches.len();
                stats.retained_capacity += watch.retained_match_capacity;
                stats.retained_dropped = stats
                    .retained_dropped
                    .saturating_add(watch.retained_dropped);
                stats
            })
    }

    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    pub fn len(&self) -> usize {
        self.watches.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catpane_core::log_entry::{LogLevel, LogPlatform};

    fn make_entry(level: LogLevel, tag: &str, message: &str) -> LogEntry {
        LogEntry {
            platform: LogPlatform::Android,
            timestamp: "01-01 00:00:00.000".to_string(),
            pid: Some(1000),
            tid: Some(1000),
            level,
            tag: tag.to_string(),
            process: None,
            subsystem: None,
            category: None,
            message: message.to_string(),
        }
    }

    fn make_buffered_entry(seq: u64, level: LogLevel, tag: &str, message: &str) -> BufferedLogEntry {
        BufferedLogEntry {
            seq,
            normalized_timestamp: None,
            entry: make_entry(level, tag, message),
        }
    }

    #[test]
    fn text_match_case_insensitive() {
        let w = Watch::new_text("t".into(), "error".into(), None, None);
        assert!(w.matches(&make_entry(LogLevel::Info, "App", "An ERROR occurred")));
        assert!(w.matches(&make_entry(LogLevel::Info, "App", "some error here")));
        assert!(!w.matches(&make_entry(LogLevel::Info, "App", "all good")));
    }

    #[test]
    fn text_match_in_tag() {
        let w = Watch::new_text("t".into(), "myapp".into(), None, None);
        assert!(w.matches(&make_entry(LogLevel::Info, "MyApp", "hello")));
    }

    #[test]
    fn regex_match() {
        let w = Watch::new_regex("r".into(), r"crash|oom", None, None).unwrap();
        assert!(w.matches(&make_entry(LogLevel::Error, "X", "app crash detected")));
        assert!(w.matches(&make_entry(LogLevel::Error, "X", "oom killer")));
        assert!(!w.matches(&make_entry(LogLevel::Error, "X", "all fine")));
    }

    #[test]
    fn tag_filter_combined_with_text() {
        let w = Watch::new_text("t".into(), "fail".into(), Some("Net".into()), None);
        assert!(w.matches(&make_entry(LogLevel::Info, "Network", "request fail")));
        assert!(!w.matches(&make_entry(LogLevel::Info, "Audio", "request fail")));
    }

    #[test]
    fn min_level_filtering() {
        let w = Watch::new_text("t".into(), "x".into(), None, Some(LogLevel::Warn));
        assert!(!w.matches(&make_entry(LogLevel::Debug, "A", "x")));
        assert!(!w.matches(&make_entry(LogLevel::Info, "A", "x")));
        assert!(w.matches(&make_entry(LogLevel::Warn, "A", "x")));
        assert!(w.matches(&make_entry(LogLevel::Error, "A", "x")));
    }

    #[test]
    fn retain_if_match_updates_stats() {
        let mut w = Watch::new_text_with_retention("t".into(), "hit".into(), None, None, 8);
        assert_eq!(w.match_count, 0);
        assert!(w.last_match_seq.is_none());

        assert!(w.retain_if_match(&make_buffered_entry(10, LogLevel::Info, "A", "hit me")));
        assert_eq!(w.match_count, 1);
        assert_eq!(w.last_match_seq, Some(10));

        assert!(!w.retain_if_match(&make_buffered_entry(11, LogLevel::Info, "A", "miss")));
        assert_eq!(w.match_count, 1);

        assert!(w.retain_if_match(&make_buffered_entry(20, LogLevel::Info, "A", "hit again")));
        assert_eq!(w.match_count, 2);
        assert_eq!(w.last_match_seq, Some(20));
    }

    #[test]
    fn retained_matches_survive_capacity_eviction() {
        let mut w = Watch::new_text_with_retention("t".into(), "hit".into(), None, None, 2);

        assert!(w.retain_if_match(&make_buffered_entry(1, LogLevel::Info, "A", "hit one")));
        assert!(w.retain_if_match(&make_buffered_entry(2, LogLevel::Info, "A", "hit two")));
        assert!(w.retain_if_match(&make_buffered_entry(3, LogLevel::Info, "A", "hit three")));

        let retained = w.retained_matches_since(None, 10);
        assert_eq!(retained.iter().map(|entry| entry.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(w.summary().retained_dropped, 1);
    }

    #[test]
    fn clear_matches_resets_retained_state() {
        let mut w = Watch::new_text_with_retention("t".into(), "hit".into(), None, None, 2);
        assert!(w.retain_if_match(&make_buffered_entry(1, LogLevel::Info, "A", "hit")));

        assert_eq!(w.clear_matches(), 1);
        let summary = w.summary();
        assert_eq!(summary.match_count, 0);
        assert!(summary.last_match_seq.is_none());
        assert_eq!(summary.retained_count, 0);
    }

    #[test]
    fn watchset_add_remove_list() {
        let mut set = WatchSet::new();
        assert!(set.is_empty());

        let w1 = Watch::new_text("first".into(), "a".into(), None, None);
        let id1 = set.add(w1);

        let w2 = Watch::new_text("second".into(), "b".into(), None, None);
        let id2 = set.add(w2);

        assert_eq!(set.len(), 2);
        assert!(set.get(&id1).is_some());
        assert!(set.get(&id2).is_some());

        let summaries = set.list();
        assert_eq!(summaries.len(), 2);

        assert!(set.remove(&id1));
        assert_eq!(set.len(), 1);
        assert!(set.get(&id1).is_none());

        assert!(!set.remove("nonexistent"));
    }

    #[test]
    fn invalid_regex_returns_error() {
        let result = Watch::new_regex("bad".into(), "[invalid", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid regex"));
    }
}
