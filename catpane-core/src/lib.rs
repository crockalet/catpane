pub mod adb;
pub mod capture;
pub mod command;
pub mod crash_detector;
pub mod filter;
pub mod ios;
pub mod ios_device;
pub mod log_buffer_config;
pub mod log_entry;
pub mod network_condition;

pub use capture::{CaptureController, CaptureHandle, CaptureScope, ConnectedDevice, DevicePlatform};
pub use crash_detector::{CrashDetector, CrashReport, CrashType, detect_crashes};
pub use filter::{Filter, TagFilter, TagLevelMatcher};
pub use ios::IosSimulator;
pub use ios_device::IosDevice;
pub use log_buffer_config::{
    DEFAULT_INITIAL_LOG_BACKLOG, DEFAULT_LOG_BUFFER_CAPACITY, initial_log_backlog,
    log_buffer_capacity,
};
pub use log_entry::{
    LogEntry, LogLevel, LogPlatform, parse_ios_log_ndjson_line, parse_ios_syslog_line,
    parse_logcat_line,
};
pub use network_condition::{
    AndroidEmulatorNetworkProfile, IOS_NETWORK_THROTTLING_ENV, NetworkConditionPreset,
    ios_network_throttling_enabled, ios_network_throttling_gate_message,
};
