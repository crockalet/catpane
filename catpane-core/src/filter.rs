use crate::log_entry::{LogEntry, LogLevel, LogPlatform};
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
    pub ios_process: Option<String>,
    pub ios_subsystem: Option<String>,
    pub ios_category: Option<String>,
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

const IOS_SYSTEM_SUBSYSTEM_PREFIXES: &[&str] = &[
    "com.apple.",
    "com.apple.CoreSimulator.",
    "com.apple.WebKit.",
];

const IOS_SYSTEM_PROCESSES: &[&str] = &[
    "SpringBoard",
    "backboardd",
    "assertiond",
    "runningboardd",
    "launchd",
    "logd",
    "installd",
    "cfprefsd",
    "networkd",
    "nsurlsessiond",
    "powerd",
    "securityd",
    "symptomsd",
    "trustd",
    "wifid",
    "Simulator",
    "SimulatorTrampoline",
];

impl Default for Filter {
    fn default() -> Self {
        Self {
            min_level: LogLevel::Info,
            package: None,
            ios_process: None,
            ios_subsystem: None,
            ios_category: None,
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
            if entry.pid != Some(pid) {
                return false;
            }
        }

        if !self.matches_ios_fields(entry) {
            return false;
        }

        // Hide vendor/system noise unless the user has explicitly targeted it.
        if self.hide_vendor_noise && !self.has_explicit_tag_filters() {
            match entry.platform {
                LogPlatform::Android => {
                    if VENDOR_TAGS.iter().any(|&t| entry.tag == t) {
                        return false;
                    }
                }
                LogPlatform::Ios => {
                    if !self.has_explicit_ios_filters() && is_ios_device_log(entry) {
                        return false;
                    }
                }
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
            re.is_match(&entry.message)
                || re.is_match(&entry.tag)
                || entry
                    .process
                    .as_ref()
                    .is_some_and(|value| re.is_match(value))
                || entry
                    .subsystem
                    .as_ref()
                    .is_some_and(|value| re.is_match(value))
                || entry
                    .category
                    .as_ref()
                    .is_some_and(|value| re.is_match(value))
        } else {
            true
        }
    }

    fn matches_ios_fields(&self, entry: &LogEntry) -> bool {
        matches_optional_contains(self.ios_process.as_deref(), entry.process.as_deref())
            && matches_optional_contains(self.ios_subsystem.as_deref(), entry.subsystem.as_deref())
            && matches_optional_contains(self.ios_category.as_deref(), entry.category.as_deref())
    }

    fn has_explicit_tag_filters(&self) -> bool {
        self.tag_filters.iter().any(|f| {
            matches!(
                f,
                TagFilter::Include(_) | TagFilter::Regex(_) | TagFilter::MinLevel { .. }
            )
        })
    }

    fn has_explicit_ios_filters(&self) -> bool {
        self.ios_process
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .ios_subsystem
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || self
                .ios_category
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
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

fn matches_optional_contains(needle: Option<&str>, haystack: Option<&str>) -> bool {
    let Some(needle) = needle.map(str::trim).filter(|needle| !needle.is_empty()) else {
        return true;
    };
    let Some(haystack) = haystack else {
        return false;
    };
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn is_ios_device_log(entry: &LogEntry) -> bool {
    entry
        .subsystem
        .as_deref()
        .is_some_and(is_ios_system_subsystem)
        || entry.process.as_deref().is_some_and(is_ios_system_process)
        || (entry.process.is_none() && entry.subsystem.is_none() && entry.tag == "iOS")
}

fn is_ios_system_subsystem(subsystem: &str) -> bool {
    IOS_SYSTEM_SUBSYSTEM_PREFIXES
        .iter()
        .any(|prefix| subsystem.starts_with(prefix))
}

fn is_ios_system_process(process: &str) -> bool {
    IOS_SYSTEM_PROCESSES
        .iter()
        .any(|candidate| process.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tag: &str, level: LogLevel) -> LogEntry {
        LogEntry {
            platform: LogPlatform::Android,
            timestamp: "03-10 06:30:45.123".to_string(),
            pid: Some(1234),
            tid: Some(5678),
            level,
            tag: tag.to_string(),
            process: None,
            subsystem: None,
            category: None,
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

    #[test]
    fn matches_ios_process_subsystem_and_category_filters() {
        let mut filter = Filter::default();
        filter.ios_process = Some("MyApp".to_string());
        filter.ios_subsystem = Some("com.example".to_string());
        filter.ios_category = Some("network".to_string());

        let entry = LogEntry {
            platform: LogPlatform::Ios,
            timestamp: "03-29 13:59:40.572".to_string(),
            pid: Some(123),
            tid: Some(42),
            level: LogLevel::Info,
            tag: "com.example.app".to_string(),
            process: Some("MyApp".to_string()),
            subsystem: Some("com.example.app".to_string()),
            category: Some("networking".to_string()),
            message: "hello".to_string(),
        };

        assert!(filter.matches(&entry, None));
    }

    #[test]
    fn hides_ios_device_logs_by_default() {
        let filter = Filter::default();
        let entry = LogEntry {
            platform: LogPlatform::Ios,
            timestamp: "03-29 13:59:40.572".to_string(),
            pid: Some(123),
            tid: Some(42),
            level: LogLevel::Info,
            tag: "com.apple.SpringBoard".to_string(),
            process: Some("SpringBoard".to_string()),
            subsystem: Some("com.apple.SpringBoard".to_string()),
            category: Some("lifecycle".to_string()),
            message: "hello".to_string(),
        };

        assert!(!filter.matches(&entry, None));
    }

    #[test]
    fn keeps_ios_app_logs_visible_by_default() {
        let filter = Filter::default();
        let entry = LogEntry {
            platform: LogPlatform::Ios,
            timestamp: "03-29 13:59:40.572".to_string(),
            pid: Some(123),
            tid: Some(42),
            level: LogLevel::Info,
            tag: "com.example.app".to_string(),
            process: Some("MyApp".to_string()),
            subsystem: Some("com.example.app".to_string()),
            category: Some("network".to_string()),
            message: "hello".to_string(),
        };

        assert!(filter.matches(&entry, None));
    }

    #[test]
    fn explicit_ios_filters_disable_device_log_suppression() {
        let mut filter = Filter::default();
        filter.ios_process = Some("SpringBoard".to_string());

        let entry = LogEntry {
            platform: LogPlatform::Ios,
            timestamp: "03-29 13:59:40.572".to_string(),
            pid: Some(123),
            tid: Some(42),
            level: LogLevel::Info,
            tag: "com.apple.SpringBoard".to_string(),
            process: Some("SpringBoard".to_string()),
            subsystem: Some("com.apple.SpringBoard".to_string()),
            category: Some("lifecycle".to_string()),
            message: "hello".to_string(),
        };

        assert!(filter.matches(&entry, None));
    }

    #[test]
    fn explicit_tag_filters_disable_ios_device_log_suppression() {
        let mut filter = Filter::default();
        filter.tag_filters = Filter::parse_tag_filters("tag:com.apple.SpringBoard");

        let entry = LogEntry {
            platform: LogPlatform::Ios,
            timestamp: "03-29 13:59:40.572".to_string(),
            pid: Some(123),
            tid: Some(42),
            level: LogLevel::Info,
            tag: "com.apple.SpringBoard".to_string(),
            process: Some("SpringBoard".to_string()),
            subsystem: Some("com.apple.SpringBoard".to_string()),
            category: Some("lifecycle".to_string()),
            message: "hello".to_string(),
        };

        assert!(filter.matches(&entry, None));
    }
}
