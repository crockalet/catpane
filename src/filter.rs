use crate::log_entry::{LogEntry, LogLevel};
use regex::Regex;

#[derive(Debug, Clone)]
pub enum TagFilter {
    Include(String),
    Exclude(String),
    Regex(Regex),
    MinLevel {
        matcher: TagLevelMatcher,
        min_level: LogLevel,
    },
}

#[derive(Debug, Clone)]
pub enum TagLevelMatcher {
    Exact(String),
    Any,
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub min_level: LogLevel,
    pub package: Option<String>,
    pub tag_filters: Vec<TagFilter>,
    pub search_query: String,
    pub search_regex: Option<Regex>,
    pub hide_vendor_noise: bool,
}

// Common vendor/system tags that flood logcat and are irrelevant to app developers
const VENDOR_TAGS: &[&str] = &[
    "chatty",
    "hwservicemanager",
    "ServiceManager",
    "HidlServiceManagement",
    "SELinux",
    "storaged",
    "Zygote",
    "ActivityThread",
    "gralloc",
    "BufferQueueProducer",
    "GraphicBufferSource",
    "SurfaceFlinger",
    "InputDispatcher",
    "InputReader",
    "InputTransport",
    "MediaCodec",
    "OMXClient",
    "PlatformConfig",
    "SensorService",
    "SensorManager",
    "netd",
    "resolv",
    "DnsProxyListener",
    "cutils-trace",
    "Looper",
    "PropertyManager",
    "vndksupport",
    "linker",
    "libc",
    "StatusBarIconController",
    "PhoneStatusBarPolicy",
    "cnss-daemon",
    "wificond",
    "WifiHAL",
    "audit",
    "thermal_repeater",
    "ThermalEngine",
    "adbd",
    "usbd",
    "lowmemorykiller",
    "lmkd",
    "perfetto",
    "traced_probes",
    "traced",
    "statsd",
];

impl Default for Filter {
    fn default() -> Self {
        Self {
            min_level: LogLevel::Info,
            package: None,
            tag_filters: Vec::new(),
            search_query: String::new(),
            search_regex: None,
            hide_vendor_noise: true,
        }
    }
}

impl Filter {
    /// Parse tag filter string using Android Studio syntax.
    pub fn parse_tag_filters(input: &str) -> Vec<TagFilter> {
        let mut filters = Vec::new();
        let mut remaining = input;

        while !remaining.is_empty() {
            remaining = remaining.trim_start();
            if remaining.is_empty() {
                break;
            }

            if let Some(rest) = remaining.strip_prefix("tag-:") {
                let (value, next) = Self::extract_tag_value(rest);
                if !value.is_empty() {
                    filters.push(TagFilter::Exclude(value));
                }
                remaining = next;
            } else if let Some(rest) = remaining.strip_prefix("tag~:") {
                let (value, next) = Self::extract_tag_value(rest);
                if !value.is_empty() {
                    if let Ok(re) = Regex::new(&value) {
                        filters.push(TagFilter::Regex(re));
                    }
                }
                remaining = next;
            } else if let Some(rest) = remaining.strip_prefix("tag:") {
                let (value, next) = Self::extract_tag_value(rest);
                if !value.is_empty() {
                    filters.push(TagFilter::Include(value));
                }
                remaining = next;
            } else {
                let (token, next) = Self::extract_token(remaining);
                if let Some((matcher, min_level)) = Self::parse_min_level_filter(token) {
                    filters.push(TagFilter::MinLevel { matcher, min_level });
                }
                remaining = next;
            }
        }

        filters
    }

    fn extract_tag_value(s: &str) -> (String, &str) {
        let patterns = [" tag-:", " tag~:", " tag:"];
        let mut min_pos = s.len();
        for pat in &patterns {
            if let Some(pos) = s.find(pat) {
                min_pos = min_pos.min(pos);
            }
        }
        (s[..min_pos].to_string(), &s[min_pos..])
    }

    fn extract_token(s: &str) -> (&str, &str) {
        if let Some(pos) = s.find(char::is_whitespace) {
            (&s[..pos], &s[pos..])
        } else {
            (s, "")
        }
    }

    fn parse_min_level_filter(token: &str) -> Option<(TagLevelMatcher, LogLevel)> {
        let (tag, level) = token.rsplit_once(':')?;
        if tag.is_empty() || level.len() != 1 {
            return None;
        }

        let min_level = LogLevel::from_char(level.chars().next()?.to_ascii_uppercase())?;
        let matcher = if tag == "*" {
            TagLevelMatcher::Any
        } else {
            TagLevelMatcher::Exact(tag.to_string())
        };

        Some((matcher, min_level))
    }

    pub fn set_search(&mut self, query: &str) {
        self.search_query = query.to_string();
        if query.is_empty() {
            self.search_regex = None;
        } else {
            self.search_regex = Regex::new(&format!("(?i){}", regex::escape(query))).ok();
        }
    }

    pub fn matches(&self, entry: &LogEntry, pid_filter: Option<u32>) -> bool {
        if entry.level < self.effective_min_level(entry) {
            return false;
        }

        if let Some(pid) = pid_filter {
            if entry.pid != pid {
                return false;
            }
        }

        // Hide vendor/system noise unless user has explicit tag include filters
        if self.hide_vendor_noise
            && !self.tag_filters.iter().any(|f| {
                matches!(
                    f,
                    TagFilter::Include(_) | TagFilter::Regex(_) | TagFilter::MinLevel { .. }
                )
            })
        {
            if VENDOR_TAGS.iter().any(|&t| entry.tag == t) {
                return false;
            }
        }

        if !self.tag_filters.is_empty() {
            let has_include = self
                .tag_filters
                .iter()
                .any(|f| matches!(f, TagFilter::Include(_) | TagFilter::Regex(_)));

            for f in &self.tag_filters {
                if let TagFilter::Exclude(name) = f {
                    if entry.tag == *name {
                        return false;
                    }
                }
            }

            if has_include {
                let any_match = self.tag_filters.iter().any(|f| match f {
                    TagFilter::Include(name) => entry.tag == *name,
                    TagFilter::Regex(re) => re.is_match(&entry.tag),
                    TagFilter::Exclude(_) | TagFilter::MinLevel { .. } => false,
                });
                if !any_match {
                    return false;
                }
            }
        }

        true
    }

    pub fn matches_search(&self, entry: &LogEntry) -> bool {
        if let Some(ref re) = self.search_regex {
            re.is_match(&entry.message) || re.is_match(&entry.tag)
        } else {
            true
        }
    }

    fn effective_min_level(&self, entry: &LogEntry) -> LogLevel {
        let mut wildcard_min_level = None;
        let mut exact_min_level = None;

        for filter in &self.tag_filters {
            if let TagFilter::MinLevel { matcher, min_level } = filter {
                match matcher {
                    TagLevelMatcher::Exact(tag) if entry.tag == *tag => {
                        exact_min_level = Some(*min_level);
                    }
                    TagLevelMatcher::Any => {
                        wildcard_min_level = Some(*min_level);
                    }
                    TagLevelMatcher::Exact(_) => {}
                }
            }
        }

        exact_min_level
            .or(wildcard_min_level)
            .unwrap_or(self.min_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tag: &str, level: LogLevel) -> LogEntry {
        LogEntry {
            timestamp: "03-10 06:30:45.123".to_string(),
            pid: 1234,
            tid: 5678,
            level,
            tag: tag.to_string(),
            message: "hello".to_string(),
        }
    }

    #[test]
    fn parses_android_style_tag_level_filters() {
        let filters = Filter::parse_tag_filters("CallManagerService:V *:E tag-:Noise");

        assert!(filters.iter().any(|filter| {
            matches!(
                filter,
                TagFilter::MinLevel {
                    matcher: TagLevelMatcher::Exact(tag),
                    min_level: LogLevel::Verbose,
                } if tag == "CallManagerService"
            )
        }));
        assert!(filters.iter().any(|filter| {
            matches!(
                filter,
                TagFilter::MinLevel {
                    matcher: TagLevelMatcher::Any,
                    min_level: LogLevel::Error,
                }
            )
        }));
        assert!(
            filters
                .iter()
                .any(|filter| matches!(filter, TagFilter::Exclude(tag) if tag == "Noise"))
        );
    }

    #[test]
    fn matches_specific_tag_and_wildcard_level_filters() {
        let mut filter = Filter {
            min_level: LogLevel::Info,
            ..Filter::default()
        };
        filter.tag_filters = Filter::parse_tag_filters("CallManagerService:V *:E");

        assert!(filter.matches(&entry("CallManagerService", LogLevel::Debug), None));
        assert!(filter.matches(&entry("OtherTag", LogLevel::Error), None));
        assert!(!filter.matches(&entry("OtherTag", LogLevel::Warn), None));
    }

    #[test]
    fn tag_level_filters_disable_vendor_noise_suppression() {
        let mut filter = Filter::default();
        filter.tag_filters = Filter::parse_tag_filters("*:E");

        assert!(filter.matches(&entry("chatty", LogLevel::Error), None));
    }
}
