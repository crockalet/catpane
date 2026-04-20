use std::{fmt, sync::OnceLock};

use regex::Regex;
use serde::Deserialize;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum LogLevel {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    pub const ALL: [LogLevel; 6] = [
        Self::Verbose,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
        Self::Fatal,
    ];

    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'V' => Some(Self::Verbose),
            'D' => Some(Self::Debug),
            'I' => Some(Self::Info),
            'W' => Some(Self::Warn),
            'E' => Some(Self::Error),
            'F' => Some(Self::Fatal),
            _ => None,
        }
    }

    pub fn as_char(self) -> char {
        match self {
            Self::Verbose => 'V',
            Self::Debug => 'D',
            Self::Info => 'I',
            Self::Warn => 'W',
            Self::Error => 'E',
            Self::Fatal => 'F',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Verbose => "Verbose",
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warn => "Warn",
            Self::Error => "Error",
            Self::Fatal => "Fatal",
        }
    }

    #[cfg(feature = "egui")]
    pub fn color(self) -> egui::Color32 {
        match self {
            Self::Verbose => egui::Color32::from_rgb(92, 99, 112),
            Self::Debug => egui::Color32::from_rgb(97, 175, 239),
            Self::Info => egui::Color32::from_rgb(152, 195, 121),
            Self::Warn => egui::Color32::from_rgb(229, 192, 123),
            Self::Error => egui::Color32::from_rgb(224, 108, 117),
            Self::Fatal => egui::Color32::from_rgb(198, 120, 221),
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_char())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogPlatform {
    Android,
    Ios,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub platform: LogPlatform,
    pub timestamp: String,
    pub pid: Option<u32>,
    pub tid: Option<u64>,
    pub level: LogLevel,
    pub tag: String,
    pub process: Option<String>,
    pub subsystem: Option<String>,
    pub category: Option<String>,
    pub message: String,
}

/// Parse a logcat -v threadtime line.
/// Format: "MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG     : message"
pub fn parse_logcat_line(line: &str) -> Option<LogEntry> {
    if line.len() < 33 {
        return None;
    }

    let timestamp = line.get(0..18)?.trim().to_string();
    let rest = line.get(18..)?.trim_start();

    let (pid_str, rest) = split_first_whitespace(rest)?;
    let pid: u32 = pid_str.parse().ok()?;

    let (tid_str, rest) = split_first_whitespace(rest)?;
    let tid: u32 = tid_str.parse().ok()?;

    let (level_str, rest) = split_first_whitespace(rest)?;
    let level = LogLevel::from_char(level_str.chars().next()?)?;

    let (tag, message) = if let Some(colon_pos) = rest.find(": ") {
        (
            rest[..colon_pos].trim().to_string(),
            rest[colon_pos + 2..].to_string(),
        )
    } else {
        (rest.trim().to_string(), String::new())
    };

    Some(LogEntry {
        platform: LogPlatform::Android,
        timestamp,
        pid: Some(pid),
        tid: Some(u64::from(tid)),
        level,
        tag,
        process: None,
        subsystem: None,
        category: None,
        message,
    })
}

fn split_first_whitespace(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace)?;
    Some((&s[..end], &s[end..]))
}

#[derive(Debug, Deserialize)]
struct IosLogRecord {
    #[serde(default, rename = "timestamp")]
    timestamp: String,
    #[serde(default, rename = "messageType")]
    message_type: String,
    #[serde(default, rename = "eventMessage")]
    event_message: String,
    #[serde(default, rename = "subsystem")]
    subsystem: String,
    #[serde(default, rename = "category")]
    category: String,
    #[serde(default, rename = "processImagePath")]
    process_image_path: String,
    #[serde(default, rename = "processID")]
    process_id: u32,
    #[serde(default, rename = "threadID")]
    thread_id: u64,
}

pub fn parse_ios_log_ndjson_line(line: &str) -> Option<LogEntry> {
    let record: IosLogRecord = serde_json::from_str(line).ok()?;
    let timestamp = normalize_ios_timestamp(&record.timestamp)?;
    let process = process_name_from_path(&record.process_image_path);
    let subsystem = normalize_optional_string(&record.subsystem);
    let category = normalize_optional_string(&record.category);
    let tag = subsystem
        .clone()
        .or_else(|| process.clone())
        .or_else(|| category.clone())
        .unwrap_or_else(|| "iOS".to_string());

    Some(LogEntry {
        platform: LogPlatform::Ios,
        timestamp,
        pid: Some(record.process_id),
        tid: Some(record.thread_id),
        level: ios_message_type_to_level(&record.message_type),
        tag,
        process,
        subsystem,
        category,
        message: record.event_message,
    })
}

pub fn parse_ios_syslog_line(line: &str) -> Option<LogEntry> {
    static IOS_SYSLOG_RE: OnceLock<Regex> = OnceLock::new();
    let regex = IOS_SYSLOG_RE.get_or_init(|| {
        Regex::new(
            r"^(?P<month>[A-Z][a-z]{2})\s+(?P<day>\d{1,2})\s+(?P<time>\d{2}:\d{2}:\d{2})(?:\.(?P<fraction>\d{1,6}))?\s+(?:\S+\s+)?(?P<process>.+?)\[(?P<pid>\d+)\]\s+<(?P<level>[^>]+)>:\s*(?P<message>.*)$",
        )
        .expect("valid iOS syslog regex")
    });
    let captures = regex.captures(line)?;

    let timestamp = normalize_ios_syslog_timestamp(
        captures.name("month")?.as_str(),
        captures.name("day")?.as_str(),
        captures.name("time")?.as_str(),
        captures.name("fraction").map(|value| value.as_str()),
    )?;
    let process = normalize_optional_string(captures.name("process")?.as_str());
    let pid = captures.name("pid")?.as_str().parse().ok();
    let message = captures.name("message")?.as_str().to_string();

    Some(LogEntry {
        platform: LogPlatform::Ios,
        timestamp,
        pid,
        tid: None,
        level: ios_syslog_level_to_level(captures.name("level")?.as_str()),
        tag: process.clone().unwrap_or_else(|| "iOS".to_string()),
        process,
        subsystem: None,
        category: None,
        message,
    })
}

fn normalize_ios_timestamp(raw: &str) -> Option<String> {
    if raw.len() < 23 {
        return None;
    }
    let month = raw.get(5..7)?;
    let day = raw.get(8..10)?;
    let time = raw.get(11..23)?;
    Some(format!("{month}-{day} {time}"))
}

fn normalize_ios_syslog_timestamp(
    month: &str,
    day: &str,
    time: &str,
    fraction: Option<&str>,
) -> Option<String> {
    let month = match month {
        "Jan" => "01",
        "Feb" => "02",
        "Mar" => "03",
        "Apr" => "04",
        "May" => "05",
        "Jun" => "06",
        "Jul" => "07",
        "Aug" => "08",
        "Sep" => "09",
        "Oct" => "10",
        "Nov" => "11",
        "Dec" => "12",
        _ => return None,
    };
    let day = format!("{:02}", day.parse::<u8>().ok()?);
    let millis = match fraction {
        Some(value) if value.len() >= 3 => value[..3].to_string(),
        Some(value) => format!("{value:0<3}"),
        None => "000".to_string(),
    };
    Some(format!("{month}-{day} {time}.{millis}"))
}

fn process_name_from_path(path: &str) -> Option<String> {
    normalize_optional_string(path)
        .and_then(|path| path.rsplit('/').next().map(str::to_string))
        .and_then(|path| normalize_optional_string(&path))
}

fn normalize_optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn ios_message_type_to_level(message_type: &str) -> LogLevel {
    match message_type.trim().to_ascii_lowercase().as_str() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "error" => LogLevel::Error,
        "fault" => LogLevel::Fatal,
        "default" | "notice" => LogLevel::Info,
        "warning" => LogLevel::Warn,
        _ => LogLevel::Verbose,
    }
}

fn ios_syslog_level_to_level(message_type: &str) -> LogLevel {
    match message_type.trim().to_ascii_lowercase().as_str() {
        "debug" => LogLevel::Debug,
        "info" | "notice" => LogLevel::Info,
        "warning" | "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        "fault" | "critical" => LogLevel::Fatal,
        _ => LogLevel::Verbose,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_threadtime() {
        let line = "03-10 06:30:45.123  1234  5678 D MyTag   : Hello world";
        let entry = parse_logcat_line(line).unwrap();
        assert_eq!(entry.timestamp, "03-10 06:30:45.123");
        assert_eq!(entry.pid, Some(1234));
        assert_eq!(entry.tid, Some(5678));
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.tag, "MyTag");
        assert_eq!(entry.message, "Hello world");
    }

    #[test]
    fn parses_ios_ndjson_lines() {
        let line = r#"{"messageType":"Default","subsystem":"com.example.app","category":"network","threadID":42,"processImagePath":"/Applications/MyApp.app/MyApp","timestamp":"2026-03-29 13:59:40.572987+0500","eventMessage":"hello from ios","processID":123}"#;
        let entry = parse_ios_log_ndjson_line(line).unwrap();
        assert_eq!(entry.platform, LogPlatform::Ios);
        assert_eq!(entry.timestamp, "03-29 13:59:40.572");
        assert_eq!(entry.pid, Some(123));
        assert_eq!(entry.tid, Some(42));
        assert_eq!(entry.tag, "com.example.app");
        assert_eq!(entry.process.as_deref(), Some("MyApp"));
        assert_eq!(entry.subsystem.as_deref(), Some("com.example.app"));
        assert_eq!(entry.category.as_deref(), Some("network"));
        assert_eq!(entry.message, "hello from ios");
    }

    #[test]
    fn parses_ios_syslog_lines() {
        // Old syslog_relay format (with hostname)
        let line = "Apr 16 12:11:32 iPhone SpringBoard[58] <Notice>: Application launched successfully";
        let entry = parse_ios_syslog_line(line).unwrap();
        assert_eq!(entry.platform, LogPlatform::Ios);
        assert_eq!(entry.timestamp, "04-16 12:11:32.000");
        assert_eq!(entry.pid, Some(58));
        assert_eq!(entry.tid, None);
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.tag, "SpringBoard");
        assert_eq!(entry.process.as_deref(), Some("SpringBoard"));
        assert_eq!(entry.subsystem, None);
        assert_eq!(entry.category, None);
        assert_eq!(entry.message, "Application launched successfully");
    }

    #[test]
    fn parses_ios_os_trace_relay_lines() {
        // Modern os_trace_relay format (no hostname, microsecond fractions)
        let line = "Apr 20 17:22:28.425237 locationd[1868] <Debug>: [Accelerometer] x,y,z";
        let entry = parse_ios_syslog_line(line).unwrap();
        assert_eq!(entry.timestamp, "04-20 17:22:28.425");
        assert_eq!(entry.pid, Some(1868));
        assert_eq!(entry.tag, "locationd");
        assert_eq!(entry.process.as_deref(), Some("locationd"));
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.message, "[Accelerometer] x,y,z");

        // os_trace_relay with parenthesized library name
        let line = "Apr 20 17:22:28.438195 SpringBoard(CoreFoundation)[35] <Debug>: Bundle: value";
        let entry = parse_ios_syslog_line(line).unwrap();
        assert_eq!(entry.pid, Some(35));
        assert_eq!(entry.tag, "SpringBoard(CoreFoundation)");
        assert_eq!(entry.process.as_deref(), Some("SpringBoard(CoreFoundation)"));
        assert_eq!(entry.message, "Bundle: value");
    }
}
