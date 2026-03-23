use crate::log_entry::{LogEntry, LogLevel};
use regex::Regex;

#[derive(Debug, Clone)]
pub enum TagFilter {
    Include(String),
    Exclude(String),
    Regex(Regex),
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
                if let Some(pos) = remaining.find(char::is_whitespace) {
                    remaining = &remaining[pos..];
                } else {
                    break;
                }
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

    pub fn set_search(&mut self, query: &str) {
        self.search_query = query.to_string();
        if query.is_empty() {
            self.search_regex = None;
        } else {
            self.search_regex = Regex::new(&format!("(?i){}", regex::escape(query))).ok();
        }
    }

    pub fn matches(&self, entry: &LogEntry, pid_filter: Option<u32>) -> bool {
        if entry.level < self.min_level {
            return false;
        }

        if let Some(pid) = pid_filter {
            if entry.pid != pid {
                return false;
            }
        }

        // Hide vendor/system noise unless user has explicit tag include filters
        if self.hide_vendor_noise
            && !self
                .tag_filters
                .iter()
                .any(|f| matches!(f, TagFilter::Include(_) | TagFilter::Regex(_)))
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
                    TagFilter::Exclude(_) => false,
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
}
