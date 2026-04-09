use catpane_core::log_entry::{LogEntry, LogLevel};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);

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
}

impl Watch {
    pub fn new_text(
        name: String,
        pattern: String,
        tag: Option<String>,
        min_level: Option<LogLevel>,
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
        }
    }

    pub fn new_regex(
        name: String,
        pattern: &str,
        tag: Option<String>,
        min_level: Option<LogLevel>,
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

    /// Test + update stats. Returns true if matched.
    pub fn check(&mut self, seq: u64, entry: &LogEntry) -> bool {
        if self.matches(entry) {
            self.match_count += 1;
            self.last_match_seq = Some(seq);
            true
        } else {
            false
        }
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
        }
    }

    pub fn pattern_display(&self) -> &str {
        self.text_pattern.as_deref().unwrap_or("")
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
    fn check_updates_stats() {
        let mut w = Watch::new_text("t".into(), "hit".into(), None, None);
        assert_eq!(w.match_count, 0);
        assert!(w.last_match_seq.is_none());

        assert!(w.check(10, &make_entry(LogLevel::Info, "A", "hit me")));
        assert_eq!(w.match_count, 1);
        assert_eq!(w.last_match_seq, Some(10));

        assert!(!w.check(11, &make_entry(LogLevel::Info, "A", "miss")));
        assert_eq!(w.match_count, 1);

        assert!(w.check(20, &make_entry(LogLevel::Info, "A", "hit again")));
        assert_eq!(w.match_count, 2);
        assert_eq!(w.last_match_seq, Some(20));
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
