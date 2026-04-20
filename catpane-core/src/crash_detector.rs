use crate::log_entry::{LogEntry, LogLevel, LogPlatform};

const MAX_ACCUMULATION_LINES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashType {
    JavaException,
    NativeCrash,
    Anr,
    IosCrash,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrashReport {
    pub crash_type: CrashType,
    pub headline: String,
    pub stack_trace: Vec<String>,
    pub first_index: usize,
    pub last_index: usize,
    pub timestamp: String,
    pub pid: Option<u32>,
    pub tag: String,
}

struct PendingCrash {
    crash_type: CrashType,
    headline: String,
    lines: Vec<String>,
    first_index: usize,
    last_index: usize,
    timestamp: String,
    pid: Option<u32>,
    tag: String,
}

impl PendingCrash {
    fn into_report(self) -> CrashReport {
        CrashReport {
            crash_type: self.crash_type,
            headline: self.headline,
            stack_trace: self.lines,
            first_index: self.first_index,
            last_index: self.last_index,
            timestamp: self.timestamp,
            pid: self.pid,
            tag: self.tag,
        }
    }
}

enum DetectorState {
    Idle,
    Accumulating(PendingCrash),
}

pub struct CrashDetector {
    state: DetectorState,
}

impl CrashDetector {
    pub fn new() -> Self {
        Self {
            state: DetectorState::Idle,
        }
    }

    /// Feed a log entry. Returns `Some(CrashReport)` when a previously
    /// accumulating crash is finalized (the current entry did not continue it).
    pub fn feed(&mut self, index: usize, entry: &LogEntry) -> Option<CrashReport> {
        match std::mem::replace(&mut self.state, DetectorState::Idle) {
            DetectorState::Idle => {
                if let Some(pending) = try_start_crash(index, entry) {
                    self.state = DetectorState::Accumulating(pending);
                }
                None
            }
            DetectorState::Accumulating(mut pending) => {
                if pending.lines.len() < MAX_ACCUMULATION_LINES && continues_crash(&pending, entry)
                {
                    pending.lines.push(entry.message.clone());
                    pending.last_index = index;
                    self.state = DetectorState::Accumulating(pending);
                    None
                } else {
                    let report = pending.into_report();
                    // The current entry might start a new crash
                    if let Some(new_pending) = try_start_crash(index, entry) {
                        self.state = DetectorState::Accumulating(new_pending);
                    }
                    Some(report)
                }
            }
        }
    }

    /// Emit any pending crash report (call after the last entry).
    pub fn flush(&mut self) -> Option<CrashReport> {
        match std::mem::replace(&mut self.state, DetectorState::Idle) {
            DetectorState::Idle => None,
            DetectorState::Accumulating(pending) => Some(pending.into_report()),
        }
    }
}

impl Default for CrashDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Trigger detection
// ---------------------------------------------------------------------------

fn try_start_crash(index: usize, entry: &LogEntry) -> Option<PendingCrash> {
    match entry.platform {
        LogPlatform::Android => try_start_android(index, entry),
        LogPlatform::Ios => try_start_ios(index, entry),
    }
}

fn try_start_android(index: usize, entry: &LogEntry) -> Option<PendingCrash> {
    let msg = &entry.message;

    // ANR (check before others — tag is usually ActivityManager)
    if msg.contains("ANR in") || msg.contains("Application Not Responding") {
        return Some(new_pending(CrashType::Anr, index, entry));
    }

    // Java/Kotlin fatal exception
    if msg.contains("FATAL EXCEPTION") {
        return Some(new_pending(CrashType::JavaException, index, entry));
    }

    // Java exception class at Error/Fatal level
    if matches!(entry.level, LogLevel::Error | LogLevel::Fatal) && looks_like_java_exception(msg) {
        return Some(new_pending(CrashType::JavaException, index, entry));
    }

    // Native crash (signal)
    if msg.contains("Fatal signal") {
        return Some(new_pending(CrashType::NativeCrash, index, entry));
    }
    if entry.level == LogLevel::Fatal && looks_like_signal(msg) {
        return Some(new_pending(CrashType::NativeCrash, index, entry));
    }

    None
}

fn try_start_ios(index: usize, entry: &LogEntry) -> Option<PendingCrash> {
    let msg = &entry.message;

    let is_crash_msg = msg.contains("Terminating app due to uncaught exception")
        || msg.contains("assertion failure")
        || msg.contains("precondition failure")
        || msg.contains("EXC_BAD_ACCESS")
        || msg.contains("EXC_CRASH")
        || entry.level == LogLevel::Fatal;

    if is_crash_msg {
        Some(new_pending(CrashType::IosCrash, index, entry))
    } else {
        None
    }
}

fn new_pending(crash_type: CrashType, index: usize, entry: &LogEntry) -> PendingCrash {
    PendingCrash {
        crash_type,
        headline: entry.message.clone(),
        lines: vec![entry.message.clone()],
        first_index: index,
        last_index: index,
        timestamp: entry.timestamp.clone(),
        pid: entry.pid,
        tag: entry.tag.clone(),
    }
}

// ---------------------------------------------------------------------------
// Continuation logic
// ---------------------------------------------------------------------------

fn continues_crash(pending: &PendingCrash, entry: &LogEntry) -> bool {
    // PID must match if both are present
    if let (Some(crash_pid), Some(entry_pid)) = (pending.pid, entry.pid) {
        if crash_pid != entry_pid {
            return false;
        }
    }

    match pending.crash_type {
        CrashType::JavaException => continues_java(entry),
        CrashType::NativeCrash => continues_native(entry),
        CrashType::Anr => continues_anr(pending, entry),
        CrashType::IosCrash => continues_ios(pending, entry),
    }
}

fn continues_java(entry: &LogEntry) -> bool {
    let msg = &entry.message;
    let trimmed = msg.trim_start();

    trimmed.starts_with("at ")
        || msg.starts_with("\tat ")
        || trimmed.starts_with("Caused by:")
        || looks_like_more_line(trimmed)
        || looks_like_java_exception(trimmed)
        || (matches!(entry.level, LogLevel::Error | LogLevel::Fatal)
            && entry.tag == "AndroidRuntime"
            && !msg.is_empty())
}

fn continues_native(entry: &LogEntry) -> bool {
    let msg = &entry.message;
    let tag = &entry.tag;
    let is_crash_tag =
        tag == "DEBUG" || tag == "crash_dump" || tag == "tombstoned" || tag == "libc";

    if !is_crash_tag {
        return false;
    }

    msg.contains("#")
        || msg.contains("backtrace:")
        || msg.contains("signal")
        || msg.contains("pc ")
        || msg.contains("fault addr")
        || msg.contains("Abort message")
        || msg.contains("pid:")
}

fn continues_anr(pending: &PendingCrash, entry: &LogEntry) -> bool {
    // ANR traces are short — allow only a few continuation lines
    if pending.lines.len() >= 3 {
        return false;
    }
    let tag = &entry.tag;
    tag == "ActivityManager" || tag == &pending.tag
}

fn continues_ios(pending: &PendingCrash, entry: &LogEntry) -> bool {
    // Match on same process name if available
    if let (Some(crash_proc), Some(entry_proc)) = (&pending_process(pending), &entry.process) {
        if crash_proc != entry_proc {
            return false;
        }
    }

    let msg = &entry.message;
    let trimmed = msg.trim_start();

    // Stack frame patterns
    trimmed.starts_with("0x")
        || trimmed.contains("0x")
        || msg.contains("CoreFoundation")
        || msg.contains("libobjc")
        || msg.contains("UIKitCore")
        || msg.contains("libsystem")
        || msg.contains("___crash")
        || msg.contains("fatal error")
        || msg.contains("Thread ")
        || looks_like_numeric_frame(trimmed)
        || entry.level == LogLevel::Fatal
}

/// Extract the process from the first pending entry by looking at the tag as a proxy.
/// iOS entries have `process` in LogEntry; we store the tag when the crash started.
fn pending_process(pending: &PendingCrash) -> Option<String> {
    // We don't store the process field separately; PID matching is sufficient.
    // Return None so we rely on PID matching from continues_crash.
    let _ = pending;
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Matches patterns like `java.lang.NullPointerException` or `kotlin.KotlinNullPointerException`.
fn looks_like_java_exception(msg: &str) -> bool {
    // Look for a qualified class name ending in Exception or Error
    let trimmed = msg.trim();
    let first_token = trimmed.split(&[' ', ':', '\t'][..]).next().unwrap_or("");
    // Must contain a dot and end with Exception or Error
    first_token.contains('.')
        && (first_token.ends_with("Exception") || first_token.ends_with("Error"))
}

/// Matches `... N more` lines in Java stack traces.
fn looks_like_more_line(s: &str) -> bool {
    s.starts_with("... ") && s.ends_with(" more")
}

/// Matches native signal descriptions like `signal 11 (SIGSEGV)`.
fn looks_like_signal(msg: &str) -> bool {
    msg.contains("signal ") && (msg.contains("SIG") || msg.contains("si_code"))
}

/// Matches numeric stack frame prefixes like `0   libsystem_kernel.dylib`.
fn looks_like_numeric_frame(s: &str) -> bool {
    s.chars().next().map_or(false, |c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Convenience functions
// ---------------------------------------------------------------------------

/// Scan a slice of log entries and return all detected crashes.
pub fn detect_crashes(entries: &[LogEntry]) -> Vec<CrashReport> {
    let mut detector = CrashDetector::new();
    let mut reports = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if let Some(report) = detector.feed(i, entry) {
            reports.push(report);
        }
    }
    if let Some(report) = detector.flush() {
        reports.push(report);
    }
    reports
}

/// Scan entries with explicit index offsets (for ring-buffer use).
pub fn detect_crashes_indexed(entries: &[(usize, &LogEntry)]) -> Vec<CrashReport> {
    let mut detector = CrashDetector::new();
    let mut reports = Vec::new();
    for &(idx, entry) in entries {
        if let Some(report) = detector.feed(idx, entry) {
            reports.push(report);
        }
    }
    if let Some(report) = detector.flush() {
        reports.push(report);
    }
    reports
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_entry::{LogEntry, LogLevel, LogPlatform};

    fn android_entry(level: LogLevel, tag: &str, message: &str) -> LogEntry {
        LogEntry {
            platform: LogPlatform::Android,
            timestamp: "01-01 12:00:00.000".into(),
            pid: Some(1234),
            tid: Some(1234),
            level,
            tag: tag.into(),
            process: None,
            subsystem: None,
            category: None,
            message: message.into(),
        }
    }

    fn android_entry_pid(level: LogLevel, tag: &str, message: &str, pid: u32) -> LogEntry {
        LogEntry {
            pid: Some(pid),
            ..android_entry(level, tag, message)
        }
    }

    fn ios_entry(level: LogLevel, message: &str) -> LogEntry {
        LogEntry {
            platform: LogPlatform::Ios,
            timestamp: "2024-01-01 12:00:00.000".into(),
            pid: Some(5678),
            tid: Some(5678),
            level,
            tag: String::new(),
            process: Some("MyApp".into()),
            subsystem: None,
            category: None,
            message: message.into(),
        }
    }

    #[test]
    fn simple_java_exception() {
        let entries = vec![
            android_entry(LogLevel::Error, "AndroidRuntime", "FATAL EXCEPTION: main"),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "java.lang.NullPointerException: Attempt to invoke virtual method",
            ),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "\tat com.example.App.onCreate(App.java:42)",
            ),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "\tat android.app.Activity.performCreate(Activity.java:1)",
            ),
            android_entry(LogLevel::Info, "System", "Normal log after crash"),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.crash_type, CrashType::JavaException);
        assert!(r.headline.contains("FATAL EXCEPTION"));
        assert_eq!(r.first_index, 0);
        assert_eq!(r.last_index, 3);
        assert_eq!(r.stack_trace.len(), 4);
    }

    #[test]
    fn java_exception_caused_by() {
        let entries = vec![
            android_entry(LogLevel::Error, "AndroidRuntime", "FATAL EXCEPTION: main"),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "java.lang.RuntimeException: Unable to start activity",
            ),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "\tat android.app.ActivityThread.main(ActivityThread.java:1)",
            ),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "Caused by: java.lang.NullPointerException",
            ),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "\tat com.example.Foo.bar(Foo.java:10)",
            ),
            android_entry(LogLevel::Error, "AndroidRuntime", "... 5 more"),
            android_entry(LogLevel::Info, "System", "unrelated"),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.stack_trace.len(), 6);
        assert!(r.stack_trace[3].contains("Caused by:"));
        assert!(r.stack_trace[5].contains("... 5 more"));
    }

    #[test]
    fn native_crash() {
        let entries = vec![
            android_entry(
                LogLevel::Fatal,
                "libc",
                "Fatal signal 11 (SIGSEGV), code 1, fault addr 0x0",
            ),
            android_entry(LogLevel::Fatal, "DEBUG", "pid: 1234, tid: 1234, name: main"),
            android_entry(LogLevel::Fatal, "DEBUG", "backtrace:"),
            android_entry(
                LogLevel::Fatal,
                "DEBUG",
                "    #00 pc 0x00001234  /system/lib/libc.so",
            ),
            android_entry(
                LogLevel::Fatal,
                "DEBUG",
                "    #01 pc 0x00005678  /data/app/libfoo.so",
            ),
            android_entry(LogLevel::Info, "System", "normal line"),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.crash_type, CrashType::NativeCrash);
        assert!(r.headline.contains("Fatal signal"));
        assert_eq!(r.first_index, 0);
        assert_eq!(r.last_index, 4);
    }

    #[test]
    fn anr_detection() {
        let entries = vec![
            android_entry(LogLevel::Error, "ActivityManager", "ANR in com.example.app"),
            android_entry(
                LogLevel::Error,
                "ActivityManager",
                "Reason: Input dispatching timed out",
            ),
            android_entry(LogLevel::Info, "System", "something else"),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.crash_type, CrashType::Anr);
        assert!(r.headline.contains("ANR in"));
        assert_eq!(r.first_index, 0);
        assert_eq!(r.last_index, 1);
    }

    #[test]
    fn ios_crash_detection() {
        let entries = vec![
            ios_entry(
                LogLevel::Error,
                "Terminating app due to uncaught exception 'NSInvalidArgumentException'",
            ),
            ios_entry(
                LogLevel::Error,
                "0x1a2b3c CoreFoundation __exceptionPreprocess",
            ),
            ios_entry(
                LogLevel::Error,
                "0x4d5e6f libobjc.A.dylib objc_exception_throw",
            ),
            ios_entry(LogLevel::Info, "normal ios log"),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.crash_type, CrashType::IosCrash);
        assert!(r.headline.contains("Terminating app"));
        assert_eq!(r.stack_trace.len(), 3);
    }

    #[test]
    fn multiple_crashes_in_sequence() {
        let entries = vec![
            android_entry(LogLevel::Error, "AndroidRuntime", "FATAL EXCEPTION: main"),
            android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                "\tat com.example.A.foo(A.java:1)",
            ),
            // Different PID starts a new crash section — but first, a normal line to break
            android_entry(LogLevel::Info, "System", "gap"),
            android_entry(
                LogLevel::Error,
                "ActivityManager",
                "ANR in com.example.other",
            ),
            android_entry(LogLevel::Info, "System", "done"),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].crash_type, CrashType::JavaException);
        assert_eq!(reports[1].crash_type, CrashType::Anr);
    }

    #[test]
    fn non_crash_entries_return_none() {
        let mut detector = CrashDetector::new();
        let e1 = android_entry(LogLevel::Info, "System", "All good");
        let e2 = android_entry(LogLevel::Debug, "MyApp", "Some debug output");
        let e3 = android_entry(LogLevel::Warn, "MyApp", "A warning");

        assert!(detector.feed(0, &e1).is_none());
        assert!(detector.feed(1, &e2).is_none());
        assert!(detector.feed(2, &e3).is_none());
        assert!(detector.flush().is_none());
    }

    #[test]
    fn flush_emits_pending() {
        let mut detector = CrashDetector::new();
        let trigger = android_entry(LogLevel::Error, "AndroidRuntime", "FATAL EXCEPTION: main");
        let frame = android_entry(
            LogLevel::Error,
            "AndroidRuntime",
            "\tat com.example.Crash.run(Crash.java:1)",
        );

        assert!(detector.feed(0, &trigger).is_none());
        assert!(detector.feed(1, &frame).is_none());

        let report = detector.flush().expect("should emit pending crash");
        assert_eq!(report.crash_type, CrashType::JavaException);
        assert_eq!(report.first_index, 0);
        assert_eq!(report.last_index, 1);
        assert_eq!(report.stack_trace.len(), 2);
    }

    #[test]
    fn accumulation_limit() {
        let mut entries = vec![android_entry(
            LogLevel::Error,
            "AndroidRuntime",
            "FATAL EXCEPTION: main",
        )];
        // Add 200 stack frames to hit the limit
        for i in 0..200 {
            entries.push(android_entry(
                LogLevel::Error,
                "AndroidRuntime",
                &format!("\tat com.example.Frame{}.method(Frame.java:{})", i, i),
            ));
        }
        // 201st continuation line should be rejected
        entries.push(android_entry(
            LogLevel::Error,
            "AndroidRuntime",
            "\tat com.example.Overflow.method(Overflow.java:1)",
        ));

        let reports = detect_crashes(&entries);
        // The first crash ends at 200 accumulated lines, then the 201st line
        // either starts a new crash or is ignored. We get at least one report.
        assert!(!reports.is_empty());
        let r = &reports[0];
        assert_eq!(r.stack_trace.len(), MAX_ACCUMULATION_LINES);
    }

    #[test]
    fn different_pid_breaks_accumulation() {
        let entries = vec![
            android_entry_pid(
                LogLevel::Error,
                "AndroidRuntime",
                "FATAL EXCEPTION: main",
                100,
            ),
            android_entry_pid(
                LogLevel::Error,
                "AndroidRuntime",
                "\tat com.example.Foo.bar(Foo.java:1)",
                100,
            ),
            // Different PID should break accumulation
            android_entry_pid(
                LogLevel::Info,
                "AndroidRuntime",
                "\tat com.other.Baz.qux(Baz.java:1)",
                999,
            ),
        ];

        let reports = detect_crashes(&entries);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.stack_trace.len(), 2);
        assert_eq!(r.pid, Some(100));
    }

    #[test]
    fn detect_crashes_indexed_offset() {
        let e1 = android_entry(LogLevel::Error, "AndroidRuntime", "FATAL EXCEPTION: main");
        let e2 = android_entry(
            LogLevel::Error,
            "AndroidRuntime",
            "\tat com.example.A.b(A.java:1)",
        );
        let e3 = android_entry(LogLevel::Info, "System", "normal");

        let indexed: Vec<(usize, &LogEntry)> = vec![(500, &e1), (501, &e2), (502, &e3)];
        let reports = detect_crashes_indexed(&indexed);

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].first_index, 500);
        assert_eq!(reports[0].last_index, 501);
    }
}
