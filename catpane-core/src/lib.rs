pub mod adb;
pub mod capture;
pub mod command;
pub mod filter;
pub mod ios;
pub mod log_buffer_config;
pub mod log_entry;

pub use capture::{CaptureController, CaptureHandle, ConnectedDevice, DevicePlatform};
pub use filter::{Filter, TagFilter, TagLevelMatcher};
pub use ios::IosSimulator;
pub use log_buffer_config::{
    DEFAULT_INITIAL_LOG_BACKLOG, DEFAULT_LOG_BUFFER_CAPACITY, initial_log_backlog,
    log_buffer_capacity,
};
pub use log_entry::{
    LogEntry, LogLevel, LogPlatform, parse_ios_log_ndjson_line, parse_logcat_line,
};
