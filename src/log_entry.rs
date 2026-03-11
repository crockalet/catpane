use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub pid: u32,
    pub tid: u32,
    pub level: LogLevel,
    pub tag: String,
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
        (rest[..colon_pos].trim().to_string(), rest[colon_pos + 2..].to_string())
    } else {
        (rest.trim().to_string(), String::new())
    };

    Some(LogEntry { timestamp, pid, tid, level, tag, message })
}

fn split_first_whitespace(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace)?;
    Some((&s[..end], &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_threadtime() {
        let line = "03-10 06:30:45.123  1234  5678 D MyTag   : Hello world";
        let entry = parse_logcat_line(line).unwrap();
        assert_eq!(entry.timestamp, "03-10 06:30:45.123");
        assert_eq!(entry.pid, 1234);
        assert_eq!(entry.tid, 5678);
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.tag, "MyTag");
        assert_eq!(entry.message, "Hello world");
    }
}
