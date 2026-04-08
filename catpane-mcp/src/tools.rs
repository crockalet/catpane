use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use catpane_core::{
    capture::{self, ConnectedDevice, DevicePlatform},
    log_buffer_config::DEFAULT_LOG_BUFFER_CAPACITY,
    log_entry::LogLevel,
};

use crate::{
    log_buffer::{
        BufferedLogEntry, LogBuffer, LogBufferMeta, LogPage, LogPageMeta, LogQuery, PageOrder,
    },
    protocol::{CallToolParams, CallToolResult, JsonObject, JsonSchema, Tool, ToolContent},
};

pub const TOOL_LIST_DEVICES: &str = "list_devices";
pub const TOOL_GET_LOGS: &str = "get_logs";
pub const TOOL_CLEAR_LOGS: &str = "clear_logs";
pub const TOOL_START_CAPTURE: &str = "start_capture";
pub const TOOL_STOP_CAPTURE: &str = "stop_capture";
pub const TOOL_GET_STATUS: &str = "get_status";

pub const DEFAULT_GET_LOGS_LIMIT: usize = 100;
pub const MAX_GET_LOGS_LIMIT: usize = 1_000;

const CAPTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_CAPTURE_REASON: &str = "stopped by stop_capture";
const RESTART_CAPTURE_REASON: &str = "stopped for restart";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolError {
    pub code: String,
    pub message: String,
}

impl McpToolError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new("invalid_params", message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }

    fn unknown_tool(message: impl Into<String>) -> Self {
        Self::new("unknown_tool", message)
    }

    fn into_call_tool_result(self) -> CallToolResult {
        let payload = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        let text = serde_json::to_string(&payload).unwrap_or_else(|_| {
            "{\"error\":{\"code\":\"internal\",\"message\":\"failed to serialize tool error\"}}"
                .to_owned()
        });
        CallToolResult::error([ToolContent::text(text)])
    }
}

impl fmt::Display for McpToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Error for McpToolError {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl CaptureSelector {
    pub fn new(capture_id: Option<String>, device: Option<String>) -> Self {
        Self { capture_id, device }.normalized()
    }

    pub fn is_empty(&self) -> bool {
        self.capture_id.is_none() && self.device.is_none()
    }

    fn normalized(mut self) -> Self {
        self.capture_id = normalize_optional_string(self.capture_id);
        self.device = normalize_optional_string(self.device);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueryOrder {
    Asc,
    #[default]
    Desc,
}

impl From<QueryOrder> for PageOrder {
    fn from(value: QueryOrder) -> Self {
        match value {
            QueryOrder::Asc => Self::Asc,
            QueryOrder::Desc => Self::Desc,
        }
    }
}

impl From<PageOrder> for QueryOrder {
    fn from(value: PageOrder) -> Self {
        match value {
            PageOrder::Asc => Self::Asc,
            PageOrder::Desc => Self::Desc,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListDevicesArgs {}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartCaptureArgs {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub capacity: Option<usize>,
    #[serde(default)]
    pub restart: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopCaptureArgs {
    #[serde(default)]
    pub capture_id: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
}

impl StopCaptureArgs {
    fn selector(&self) -> CaptureSelector {
        CaptureSelector::new(self.capture_id.clone(), self.device.clone())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClearLogsArgs {
    #[serde(default)]
    pub capture_id: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
}

impl ClearLogsArgs {
    fn selector(&self) -> CaptureSelector {
        CaptureSelector::new(self.capture_id.clone(), self.device.clone())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetStatusArgs {
    #[serde(default)]
    pub capture_id: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub include_devices: bool,
}

impl GetStatusArgs {
    fn selector(&self) -> CaptureSelector {
        CaptureSelector::new(self.capture_id.clone(), self.device.clone())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetLogsArgs {
    #[serde(default)]
    pub capture_id: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub cursor: Option<u64>,
    #[serde(default)]
    pub order: QueryOrder,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub min_level: Option<String>,
    #[serde(default)]
    pub tag_query: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub subsystem: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
}

impl GetLogsArgs {
    fn selector(&self) -> CaptureSelector {
        CaptureSelector::new(self.capture_id.clone(), self.device.clone())
    }

    fn into_query(self) -> Result<LogQuery, McpToolError> {
        let limit = self.limit.unwrap_or(DEFAULT_GET_LOGS_LIMIT);
        if limit > MAX_GET_LOGS_LIMIT {
            return Err(McpToolError::invalid_params(format!(
                "limit must be <= {MAX_GET_LOGS_LIMIT}"
            )));
        }

        let min_level = normalize_optional_string(self.min_level)
            .as_deref()
            .map(parse_log_level)
            .transpose()?;
        let tag_query = normalize_optional_string(self.tag_query);
        let text = normalize_optional_string(self.text);
        let process = normalize_optional_string(self.process);
        let subsystem = normalize_optional_string(self.subsystem);
        let category = normalize_optional_string(self.category);
        let since = normalize_optional_string(self.since);

        let mut query = LogQuery {
            cursor: self.cursor,
            order: self.order.into(),
            limit,
            min_level,
            tag_query: None,
            text,
            process,
            subsystem,
            category,
            since: None,
        };

        if let Some(tag_query) = tag_query.as_deref() {
            query.set_tag_query(tag_query);
        }

        if let Some(since) = since.as_deref() {
            query.set_since_str(since).map_err(|err| {
                McpToolError::invalid_params(format!("since must be MM-DD HH:MM:SS.mmm: {err}"))
            })?;
        }

        Ok(query)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub serial: String,
    pub description: String,
    pub friendly_name: String,
    pub platform: DevicePlatform,
    pub is_tcp: bool,
}

impl From<ConnectedDevice> for DeviceInfo {
    fn from(device: ConnectedDevice) -> Self {
        let is_tcp = device.supports_disconnect();
        Self {
            friendly_name: device.name,
            is_tcp,
            serial: device.id,
            description: device.description,
            platform: device.platform,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogBufferStatus {
    pub capacity: usize,
    pub len: usize,
    pub dropped: u64,
    pub next_seq: u64,
    pub oldest_seq: Option<u64>,
    pub newest_seq: Option<u64>,
}

impl From<LogBufferMeta> for LogBufferStatus {
    fn from(value: LogBufferMeta) -> Self {
        Self {
            capacity: value.capacity,
            len: value.len,
            dropped: value.dropped,
            next_seq: value.next_seq,
            oldest_seq: value.oldest_seq,
            newest_seq: value.newest_seq,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub capture_id: String,
    pub device: String,
    pub platform: DevicePlatform,
    pub device_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_filter: Option<u32>,
    pub running: bool,
    pub started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub ingested_lines: u64,
    pub parsed_entries: u64,
    pub parse_errors: u64,
    pub buffer: LogBufferStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntryView {
    pub seq: u64,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tid: Option<u64>,
    pub level: String,
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub message: String,
}

impl From<BufferedLogEntry> for LogEntryView {
    fn from(buffered: BufferedLogEntry) -> Self {
        let level = buffered.entry.level;
        let timestamp = buffered.entry.timestamp;
        let normalized_timestamp = buffered
            .normalized_timestamp
            .map(|value| value.to_string())
            .filter(|normalized| *normalized != timestamp);
        Self {
            seq: buffered.seq,
            timestamp,
            normalized_timestamp,
            pid: buffered.entry.pid,
            tid: buffered.entry.tid,
            level: level.as_char().to_string(),
            tag: buffered.entry.tag,
            process: buffered.entry.process,
            subsystem: buffered.entry.subsystem,
            category: buffered.entry.category,
            message: buffered.entry.message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPageView {
    pub cursor: Option<u64>,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub next_cursor: Option<u64>,
    pub returned: usize,
    pub limit: usize,
    pub order: QueryOrder,
    pub has_more: bool,
}

impl From<LogPageMeta> for LogPageView {
    fn from(value: LogPageMeta) -> Self {
        Self {
            cursor: value.cursor,
            first_seq: value.first_seq,
            last_seq: value.last_seq,
            next_cursor: value.next_cursor,
            returned: value.returned,
            limit: value.limit,
            order: value.order.into(),
            has_more: value.has_more,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDevicesResponse {
    pub device_count: usize,
    pub devices: Vec<DeviceInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCaptureResponse {
    pub restarted: bool,
    pub capture: CaptureStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopCaptureResponse {
    pub stopped: bool,
    pub capture: CaptureStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearLogsResponse {
    pub cleared_entries: usize,
    pub capture: CaptureStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetLogsResponse {
    pub capture: CaptureStatus,
    pub page: LogPageView,
    pub entries: Vec<LogEntryView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetStatusResponse {
    pub default_buffer_capacity: usize,
    pub capture_count: usize,
    pub running_capture_count: usize,
    pub captures: Vec<CaptureStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devices: Option<Vec<DeviceInfo>>,
}

pub fn tool_definitions() -> Vec<Tool> {
    vec![
        list_devices_tool(),
        get_logs_tool(),
        clear_logs_tool(),
        start_capture_tool(),
        stop_capture_tool(),
        get_status_tool(),
    ]
}

#[derive(Clone)]
pub struct McpRuntimeState {
    inner: Arc<Mutex<RuntimeInner>>,
    default_buffer_capacity: usize,
}

impl Default for McpRuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl McpRuntimeState {
    pub fn new() -> Self {
        Self::with_buffer_capacity(DEFAULT_LOG_BUFFER_CAPACITY)
    }

    pub fn with_buffer_capacity(default_buffer_capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeInner::default())),
            default_buffer_capacity: default_buffer_capacity.max(1),
        }
    }

    pub fn default_buffer_capacity(&self) -> usize {
        self.default_buffer_capacity
    }

    pub async fn handle_tool_call(&self, rt: &Handle, params: CallToolParams) -> CallToolResult {
        match self.dispatch_tool_call(rt, params).await {
            Ok(result) => result,
            Err(err) => err.into_call_tool_result(),
        }
    }

    pub async fn list_devices(
        &self,
        _args: ListDevicesArgs,
    ) -> Result<ListDevicesResponse, McpToolError> {
        let devices = capture::list_devices_strict()
            .await
            .map_err(McpToolError::internal)?
            .into_iter()
            .map(DeviceInfo::from)
            .collect::<Vec<_>>();
        Ok(ListDevicesResponse {
            device_count: devices.len(),
            devices,
        })
    }

    pub async fn start_capture(
        &self,
        rt: &Handle,
        args: StartCaptureArgs,
    ) -> Result<StartCaptureResponse, McpToolError> {
        let device = resolve_connected_device(args.device).await?;
        let package = normalize_optional_string(args.package);
        let pid_filter = resolve_pid_filter(&device, args.pid, package.as_deref()).await?;
        let capacity = args.capacity.unwrap_or(self.default_buffer_capacity);
        if capacity == 0 {
            return Err(McpToolError::invalid_params(
                "capacity must be greater than zero",
            ));
        }

        let restart_plan = {
            let mut inner = lock_recover(&self.inner);
            inner.prepare_start(&device, args.restart)?
        };

        if let Some(shutdown) = restart_plan.shutdown.as_ref() {
            shutdown.wait_for_restart().await?;
        }

        let (capture, restarted) = {
            let mut inner = lock_recover(&self.inner);
            inner.finalize_replaced_capture(&device, &restart_plan)?;

            let capture_id = format!("capture-{}", inner.next_capture_id);
            inner.next_capture_id = inner.next_capture_id.saturating_add(1);

            let shared = Arc::new(CaptureShared::new(
                device.id.clone(),
                device.name.clone(),
                device.platform,
                package.clone(),
                pid_filter,
                capacity,
            ));
            let mut stream = capture::spawn_capture(rt, &device, pid_filter);
            let capture_control = stream.controller();
            let (pump_done_tx, pump_done) = watch::channel(false);
            let shared_for_task = Arc::clone(&shared);

            let pump_task = rt.spawn(async move {
                let _pump_done = CompletionSignal::new(pump_done_tx);
                while let Some(entry) = stream.rx.recv().await {
                    shared_for_task.append_entry(entry);
                }
                shared_for_task.finish("capture stream ended or failed to start");
            });

            let capture = CaptureRuntime {
                capture_id: capture_id.clone(),
                shared,
                capture_control,
                pump_done,
                pump_task,
                shutdown_requested: false,
            };
            let snapshot = capture.snapshot();
            inner.captures.insert(capture_id, capture);
            (snapshot, restart_plan.replaced_capture_id.is_some())
        };

        Ok(StartCaptureResponse { restarted, capture })
    }

    pub async fn stop_capture(
        &self,
        args: StopCaptureArgs,
    ) -> Result<StopCaptureResponse, McpToolError> {
        let selector = args.selector();
        let shutdown = {
            let mut inner = lock_recover(&self.inner);
            inner.prepare_stop(&selector)?
        };
        shutdown.wait_for_stop().await?;

        let capture = {
            let mut inner = lock_recover(&self.inner);
            inner.finalize_capture_shutdown(&shutdown.capture_id)
                .ok_or_else(|| {
                    McpToolError::internal(format!(
                        "capture `{}` finished stopping but could not be removed from runtime state",
                        shutdown.capture_id
                    ))
                })?
        };
        Ok(capture.into_stopped_response(STOP_CAPTURE_REASON))
    }

    pub fn clear_logs(&self, args: ClearLogsArgs) -> Result<ClearLogsResponse, McpToolError> {
        let selector = args.selector();
        let inner = lock_recover(&self.inner);
        let capture_id = resolve_capture_id(&inner.captures, &selector)?;
        let capture = inner.captures.get(&capture_id).ok_or_else(|| {
            McpToolError::not_found(format!("capture {capture_id} was not found"))
        })?;
        Ok(capture.clear_logs())
    }

    pub fn get_logs(&self, args: GetLogsArgs) -> Result<GetLogsResponse, McpToolError> {
        let selector = args.selector();
        let inner = lock_recover(&self.inner);
        let capture_id = resolve_capture_id(&inner.captures, &selector)?;
        let capture = inner.captures.get(&capture_id).ok_or_else(|| {
            McpToolError::not_found(format!("capture {capture_id} was not found"))
        })?;
        capture.query_logs(args)
    }

    pub async fn get_status(&self, args: GetStatusArgs) -> Result<GetStatusResponse, McpToolError> {
        let selector = args.selector();
        let (capture_count, running_capture_count, captures) = {
            let inner = lock_recover(&self.inner);
            let selected_capture_id = if selector.is_empty() {
                None
            } else {
                Some(resolve_capture_id(&inner.captures, &selector)?)
            };

            let mut captures = inner
                .captures
                .values()
                .map(CaptureRuntime::snapshot)
                .collect::<Vec<_>>();
            sort_capture_statuses(&mut captures);

            let capture_count = captures.len();
            let running_capture_count = captures.iter().filter(|capture| capture.running).count();
            let captures = if let Some(selected_capture_id) = selected_capture_id {
                captures
                    .into_iter()
                    .filter(|capture| capture.capture_id == selected_capture_id)
                    .collect()
            } else {
                captures
            };

            (capture_count, running_capture_count, captures)
        };

        let devices = if args.include_devices {
            Some(self.list_devices(ListDevicesArgs::default()).await?.devices)
        } else {
            None
        };

        Ok(GetStatusResponse {
            default_buffer_capacity: self.default_buffer_capacity,
            capture_count,
            running_capture_count,
            captures,
            devices,
        })
    }

    async fn dispatch_tool_call(
        &self,
        rt: &Handle,
        params: CallToolParams,
    ) -> Result<CallToolResult, McpToolError> {
        match params.name.as_str() {
            TOOL_LIST_DEVICES => {
                let args = parse_arguments::<ListDevicesArgs>(&params)?;
                json_success(&self.list_devices(args).await?)
            }
            TOOL_GET_LOGS => {
                let args = parse_arguments::<GetLogsArgs>(&params)?;
                json_success(&self.get_logs(args)?)
            }
            TOOL_CLEAR_LOGS => {
                let args = parse_arguments::<ClearLogsArgs>(&params)?;
                json_success(&self.clear_logs(args)?)
            }
            TOOL_START_CAPTURE => {
                let args = parse_arguments::<StartCaptureArgs>(&params)?;
                json_success(&self.start_capture(rt, args).await?)
            }
            TOOL_STOP_CAPTURE => {
                let args = parse_arguments::<StopCaptureArgs>(&params)?;
                json_success(&self.stop_capture(args).await?)
            }
            TOOL_GET_STATUS => {
                let args = parse_arguments::<GetStatusArgs>(&params)?;
                json_success(&self.get_status(args).await?)
            }
            _ => Err(McpToolError::unknown_tool(format!(
                "unknown MCP tool: {}",
                params.name
            ))),
        }
    }
}

pub async fn handle_tool_call(
    rt: &Handle,
    state: &McpRuntimeState,
    params: CallToolParams,
) -> CallToolResult {
    state.handle_tool_call(rt, params).await
}

struct RuntimeInner {
    next_capture_id: u64,
    captures: HashMap<String, CaptureRuntime>,
}

impl Default for RuntimeInner {
    fn default() -> Self {
        Self {
            next_capture_id: 1,
            captures: HashMap::new(),
        }
    }
}

impl RuntimeInner {
    fn prepare_start(
        &mut self,
        device: &ConnectedDevice,
        restart: bool,
    ) -> Result<CaptureStartPlan, McpToolError> {
        let Some(existing_capture_id) = find_capture_id_by_device(&self.captures, &device.id)
        else {
            return Ok(CaptureStartPlan::default());
        };

        let existing = self.captures.get_mut(&existing_capture_id).ok_or_else(|| {
            McpToolError::internal(format!(
                "capture `{existing_capture_id}` disappeared during start preparation"
            ))
        })?;

        if existing.is_running() && !restart {
            return Err(McpToolError::conflict(format!(
                "a capture is already running for device {device}; stop it first or pass restart=true"
            )));
        }

        let shutdown = existing
            .is_running()
            .then(|| existing.request_shutdown(RESTART_CAPTURE_REASON));

        Ok(CaptureStartPlan {
            replaced_capture_id: Some(existing_capture_id),
            shutdown,
        })
    }

    fn finalize_replaced_capture(
        &mut self,
        device: &ConnectedDevice,
        plan: &CaptureStartPlan,
    ) -> Result<(), McpToolError> {
        if let Some(replaced_capture_id) = plan.replaced_capture_id.as_deref() {
            if let Some(active_capture_id) = find_capture_id_by_device(&self.captures, &device.id) {
                if active_capture_id != replaced_capture_id {
                    return Err(McpToolError::conflict(format!(
                        "another capture became active for device {device} while restart cleanup was in progress"
                    )));
                }
            }
            self.captures.remove(replaced_capture_id);
        } else if let Some(active_capture_id) =
            find_capture_id_by_device(&self.captures, &device.id)
        {
            return Err(McpToolError::conflict(format!(
                "a capture is already registered for device {device} ({active_capture_id})"
            )));
        }

        Ok(())
    }

    fn prepare_stop(
        &mut self,
        selector: &CaptureSelector,
    ) -> Result<CaptureShutdownWait, McpToolError> {
        let capture_id = resolve_capture_id(&self.captures, selector)?;
        let capture = self.captures.get_mut(&capture_id).ok_or_else(|| {
            McpToolError::not_found(format!("capture `{capture_id}` was not found"))
        })?;
        Ok(capture.request_shutdown(STOP_CAPTURE_REASON))
    }

    fn finalize_capture_shutdown(&mut self, capture_id: &str) -> Option<CaptureRuntime> {
        self.captures.remove(capture_id)
    }
}

#[derive(Debug, Default)]
struct CaptureStartPlan {
    replaced_capture_id: Option<String>,
    shutdown: Option<CaptureShutdownWait>,
}

#[derive(Debug, Clone)]
struct CaptureShutdownWait {
    capture_id: String,
    device: String,
    capture_shutdown: capture::CaptureController,
    pump_done: watch::Receiver<bool>,
}

impl CaptureShutdownWait {
    async fn wait_for_restart(&self) -> Result<(), McpToolError> {
        self.wait_with_timeout(
            CAPTURE_SHUTDOWN_TIMEOUT,
            format!(
                "waiting for existing capture `{}` on device `{}` to stop before restart",
                self.capture_id, self.device
            ),
        )
        .await
    }

    async fn wait_for_stop(&self) -> Result<(), McpToolError> {
        self.wait_with_timeout(
            CAPTURE_SHUTDOWN_TIMEOUT,
            format!(
                "waiting for capture `{}` on device `{}` to stop",
                self.capture_id, self.device
            ),
        )
        .await
    }

    async fn wait_with_timeout(
        &self,
        timeout: Duration,
        context: String,
    ) -> Result<(), McpToolError> {
        tokio::time::timeout(timeout, async {
            self.capture_shutdown.wait_for_shutdown().await;
            wait_for_completion(self.pump_done.clone()).await;
        })
        .await
        .map_err(|_| {
            McpToolError::conflict(format!(
                "timed out after {} while {context}",
                format_duration(timeout)
            ))
        })?;
        Ok(())
    }
}

struct CaptureRuntime {
    capture_id: String,
    shared: Arc<CaptureShared>,
    capture_control: capture::CaptureController,
    pump_done: watch::Receiver<bool>,
    pump_task: JoinHandle<()>,
    shutdown_requested: bool,
}

impl CaptureRuntime {
    fn request_shutdown(&mut self, reason: &str) -> CaptureShutdownWait {
        self.shared.mark_stop_requested(reason);
        if !self.shutdown_requested {
            self.capture_control.stop();
            self.shutdown_requested = true;
        }

        CaptureShutdownWait {
            capture_id: self.capture_id.clone(),
            device: self.shared.device.clone(),
            capture_shutdown: self.capture_control.clone(),
            pump_done: self.pump_done.clone(),
        }
    }

    fn device(&self) -> &str {
        &self.shared.device
    }

    fn is_running(&self) -> bool {
        !self.pump_task.is_finished()
    }

    fn snapshot(&self) -> CaptureStatus {
        self.shared.snapshot(&self.capture_id, self.is_running())
    }

    fn clear_logs(&self) -> ClearLogsResponse {
        let cleared_entries = self.shared.clear_logs();
        ClearLogsResponse {
            cleared_entries,
            capture: self.snapshot(),
        }
    }

    fn query_logs(&self, args: GetLogsArgs) -> Result<GetLogsResponse, McpToolError> {
        let query = args.into_query()?;
        let LogPage { entries, meta } = self.shared.query(&query);
        let mut capture = self.snapshot();
        capture.buffer = meta.buffer.into();

        Ok(GetLogsResponse {
            capture,
            page: meta.into(),
            entries: entries.into_iter().map(LogEntryView::from).collect(),
        })
    }

    fn into_stopped_response(self, reason: &str) -> StopCaptureResponse {
        self.shared.finish(reason);
        let mut capture = self.snapshot();
        capture.running = false;
        capture.stop_reason = Some(reason.to_owned());

        StopCaptureResponse {
            stopped: true,
            capture,
        }
    }
}

#[derive(Default)]
struct CaptureStats {
    ingested_lines: u64,
    parsed_entries: u64,
    parse_errors: u64,
    finished_at_ms: Option<u64>,
    stop_reason: Option<String>,
}

struct CaptureShared {
    device: String,
    device_name: String,
    platform: DevicePlatform,
    package: Option<String>,
    pid_filter: Option<u32>,
    started_at_ms: u64,
    buffer: Mutex<LogBuffer>,
    stats: Mutex<CaptureStats>,
}

impl CaptureShared {
    fn new(
        device: String,
        device_name: String,
        platform: DevicePlatform,
        package: Option<String>,
        pid_filter: Option<u32>,
        capacity: usize,
    ) -> Self {
        Self {
            device,
            device_name,
            platform,
            package,
            pid_filter,
            started_at_ms: now_epoch_ms(),
            buffer: Mutex::new(LogBuffer::new(capacity)),
            stats: Mutex::new(CaptureStats::default()),
        }
    }

    fn append_entry(&self, entry: catpane_core::log_entry::LogEntry) {
        {
            let mut buffer = lock_recover(&self.buffer);
            buffer.append(entry);
        }
        let mut stats = lock_recover(&self.stats);
        stats.ingested_lines = stats.ingested_lines.saturating_add(1);
        stats.parsed_entries = stats.parsed_entries.saturating_add(1);
    }

    fn clear_logs(&self) -> usize {
        let mut buffer = lock_recover(&self.buffer);
        let cleared_entries = buffer.len();
        buffer.clear();
        cleared_entries
    }

    fn query(&self, query: &LogQuery) -> LogPage {
        let buffer = lock_recover(&self.buffer);
        buffer.query(query)
    }

    fn mark_stop_requested(&self, reason: &str) {
        let mut stats = lock_recover(&self.stats);
        if stats.stop_reason.is_none() {
            stats.stop_reason = Some(reason.to_owned());
        }
    }

    fn finish(&self, reason: &str) {
        let mut stats = lock_recover(&self.stats);
        if stats.finished_at_ms.is_none() {
            stats.finished_at_ms = Some(now_epoch_ms());
        }
        if stats.stop_reason.is_none() {
            stats.stop_reason = Some(reason.to_owned());
        }
    }

    fn snapshot(&self, capture_id: &str, running: bool) -> CaptureStatus {
        let buffer = {
            let buffer = lock_recover(&self.buffer);
            LogBufferStatus::from(buffer.meta())
        };
        let stats = lock_recover(&self.stats);

        CaptureStatus {
            capture_id: capture_id.to_owned(),
            device: self.device.clone(),
            platform: self.platform,
            device_name: self.device_name.clone(),
            package: self.package.clone(),
            pid_filter: self.pid_filter,
            running,
            started_at_ms: self.started_at_ms,
            finished_at_ms: stats.finished_at_ms,
            stop_reason: stats.stop_reason.clone(),
            ingested_lines: stats.ingested_lines,
            parsed_entries: stats.parsed_entries,
            parse_errors: stats.parse_errors,
            buffer,
        }
    }
}

fn list_devices_tool() -> Tool {
    Tool::new(TOOL_LIST_DEVICES, empty_object_schema())
        .with_description("List connected Android devices and booted iOS simulators that CatPane can capture logs from.")
}

fn get_logs_tool() -> Tool {
    Tool::new(
        TOOL_GET_LOGS,
        object_schema(
            vec![
                ("captureId", string_property("Specific capture ID to query.")),
                ("device", string_property("Resolve a capture by connected device identifier.")),
                (
                    "cursor",
                    integer_property(
                        "Exclusive sequence cursor. Desc pages older entries with seq < cursor; asc pages newer entries with seq > cursor.",
                    ),
                ),
                (
                    "order",
                    json!({
                        "type": "string",
                        "enum": ["asc", "desc"],
                        "description": "Page direction. Defaults to desc so the newest logs are returned first."
                    }),
                ),
                (
                    "limit",
                    json!({
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MAX_GET_LOGS_LIMIT,
                        "description": format!("Maximum number of entries to return. Defaults to {DEFAULT_GET_LOGS_LIMIT}."),
                    }),
                ),
                (
                    "minLevel",
                    json!({
                        "type": "string",
                        "description": "Minimum log level filter. Accepts verbose/debug/info/warn/error/fatal or the single-letter V/D/I/W/E/F aliases."
                    }),
                ),
                (
                    "tagQuery",
                    string_property(
                        "CatPane tag filter syntax, for example `tag:MyTag tag-:Noise tag~:^App` or `MyTag:V *:E`.",
                    ),
                ),
                ("text", string_property("Substring search over tag and message text.")),
                ("process", string_property("Filter by process name substring (iOS simulator captures only).")),
                ("subsystem", string_property("Filter by subsystem substring (iOS simulator captures only).")),
                ("category", string_property("Filter by category substring (iOS simulator captures only).")),
                (
                    "since",
                    string_property(
                        "Only return entries at or after this threadtime timestamp: MM-DD HH:MM:SS.mmm.",
                    ),
                ),
            ],
            &[],
        ),
    )
    .with_description("Read buffered capture entries with cursor pagination and CatPane filters.")
}

fn clear_logs_tool() -> Tool {
    Tool::new(
        TOOL_CLEAR_LOGS,
        object_schema(
            vec![
                (
                    "captureId",
                    string_property("Specific capture ID to clear."),
                ),
                (
                    "device",
                    string_property("Resolve a capture by connected device identifier."),
                ),
            ],
            &[],
        ),
    )
    .with_description("Clear buffered log entries for a capture without stopping it.")
}

fn start_capture_tool() -> Tool {
    Tool::new(
        TOOL_START_CAPTURE,
        object_schema(
            vec![
                (
                    "device",
                    string_property(
                        "Connected device identifier to capture. If omitted and exactly one Android device or booted iOS simulator is available, that device is used automatically.",
                    ),
                ),
                (
                    "pid",
                    integer_property("PID filter passed through to adb logcat (Android only)."),
                ),
                (
                    "package",
                    string_property(
                        "Package name to resolve into a PID filter before starting capture (Android only).",
                    ),
                ),
                (
                    "capacity",
                    json!({
                        "type": "integer",
                        "minimum": 1,
                        "description": "Ring-buffer size for this capture. Defaults to the runtime's configured capacity."
                    }),
                ),
                (
                    "restart",
                    json!({
                        "type": "boolean",
                        "description": "Replace any existing capture already registered for the selected device."
                    }),
                ),
            ],
            &[],
        ),
    )
    .with_description("Start a new Android or iOS simulator capture and buffer it for later MCP queries.")
}

fn stop_capture_tool() -> Tool {
    Tool::new(
        TOOL_STOP_CAPTURE,
        object_schema(
            vec![
                ("captureId", string_property("Specific capture ID to stop.")),
                (
                    "device",
                    string_property("Resolve a capture by connected device identifier."),
                ),
            ],
            &[],
        ),
    )
    .with_description("Stop a running capture and remove it from the MCP runtime state.")
}

fn get_status_tool() -> Tool {
    Tool::new(
        TOOL_GET_STATUS,
        object_schema(
            vec![
                ("captureId", string_property("Specific capture ID to inspect.")),
                ("device", string_property("Resolve a capture by connected device identifier.")),
                (
                    "includeDevices",
                    json!({
                        "type": "boolean",
                        "description": "Also include the current connected Android devices and booted iOS simulators."
                    }),
                ),
            ],
            &[],
        ),
    )
    .with_description("Inspect registered captures, buffer usage, and optional connected-device state.")
}

fn empty_object_schema() -> JsonSchema {
    JsonSchema::new(json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    }))
}

fn object_schema(properties: Vec<(&'static str, Value)>, required: &[&str]) -> JsonSchema {
    let mut property_map = JsonObject::new();
    for (name, schema) in properties {
        property_map.insert(name.to_owned(), schema);
    }

    let mut schema = json!({
        "type": "object",
        "properties": property_map,
        "additionalProperties": false,
    });

    if !required.is_empty() {
        schema
            .as_object_mut()
            .expect("schema root must be an object")
            .insert("required".to_owned(), json!(required));
    }

    JsonSchema::new(schema)
}

fn string_property(description: &str) -> Value {
    json!({
        "type": "string",
        "description": description,
    })
}

fn integer_property(description: &str) -> Value {
    json!({
        "type": "integer",
        "description": description,
    })
}

fn parse_arguments<T>(params: &CallToolParams) -> Result<T, McpToolError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(Value::Object(params.arguments.clone().unwrap_or_default())).map_err(
        |err| McpToolError::invalid_params(format!("invalid arguments for {}: {err}", params.name)),
    )
}

fn parse_log_level(input: &str) -> Result<LogLevel, McpToolError> {
    let normalized = input.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "v" | "verbose" => Ok(LogLevel::Verbose),
        "d" | "debug" => Ok(LogLevel::Debug),
        "i" | "info" | "information" => Ok(LogLevel::Info),
        "w" | "warn" | "warning" => Ok(LogLevel::Warn),
        "e" | "error" => Ok(LogLevel::Error),
        "f" | "fatal" => Ok(LogLevel::Fatal),
        _ => Err(McpToolError::invalid_params(format!(
            "unsupported log level `{input}`"
        ))),
    }
}

fn json_success<T>(value: &T) -> Result<CallToolResult, McpToolError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(|err| {
        McpToolError::internal(format!("failed to serialize tool response: {err}"))
    })?;
    Ok(CallToolResult::success([ToolContent::text(text)]))
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

async fn resolve_connected_device(device: Option<String>) -> Result<ConnectedDevice, McpToolError> {
    let device = normalize_optional_string(device);
    let devices = capture::list_devices_strict()
        .await
        .map_err(McpToolError::internal)?;

    if let Some(device) = device {
        if let Some(connected) = devices.iter().find(|connected| connected.id == device) {
            return Ok(connected.clone());
        }

        if devices.is_empty() {
            return Err(McpToolError::not_found(format!(
                "device `{device}` is not connected and no Android devices or booted iOS simulators are currently available"
            )));
        }

        return Err(McpToolError::not_found(format!(
            "device `{device}` is not connected; available devices: {}",
            joined_device_serials(&devices)
        )));
    }

    match devices.as_slice() {
        [] => Err(McpToolError::not_found(
            "no connected Android devices or booted iOS simulators found; connect a device or boot a simulator",
        )),
        [device] => Ok(device.clone()),
        _ => Err(McpToolError::invalid_params(format!(
            "multiple capture targets are available; specify device explicitly ({})",
            joined_device_serials(&devices)
        ))),
    }
}

async fn resolve_pid_filter(
    device: &ConnectedDevice,
    pid: Option<u32>,
    package: Option<&str>,
) -> Result<Option<u32>, McpToolError> {
    if device.platform != DevicePlatform::Android {
        if pid.is_some() || package.is_some() {
            return Err(McpToolError::invalid_params(
                "pid and package filters are only supported for Android captures",
            ));
        }
        return Ok(None);
    }

    if let Some(pid) = pid {
        if pid == 0 {
            return Err(McpToolError::invalid_params(
                "pid must be greater than zero",
            ));
        }
        if package.is_some() {
            return Err(McpToolError::invalid_params(
                "specify either pid or package, but not both",
            ));
        }
        return Ok(Some(pid));
    }

    let Some(package) = package else {
        return Ok(None);
    };

    match capture::get_pid_for_package_strict(&device.id, package, std::slice::from_ref(device))
        .await
        .map_err(McpToolError::internal)?
    {
        Some(pid) => Ok(Some(pid)),
        None => Err(McpToolError::not_found(format!(
            "could not resolve a PID for package `{package}` on device `{}`",
            device.id
        ))),
    }
}

fn joined_device_serials(devices: &[ConnectedDevice]) -> String {
    devices
        .iter()
        .map(|device| device.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn find_capture_id_by_device(
    captures: &HashMap<String, CaptureRuntime>,
    device: &str,
) -> Option<String> {
    captures
        .values()
        .find(|capture| capture.device() == device)
        .map(|capture| capture.capture_id.clone())
}

fn resolve_capture_id(
    captures: &HashMap<String, CaptureRuntime>,
    selector: &CaptureSelector,
) -> Result<String, McpToolError> {
    let selector = selector.clone().normalized();
    match (&selector.capture_id, &selector.device) {
        (Some(capture_id), Some(device)) => {
            let capture = captures.get(capture_id).ok_or_else(|| {
                McpToolError::not_found(format!("capture `{capture_id}` was not found"))
            })?;
            if capture.device() != device {
                return Err(McpToolError::invalid_params(format!(
                    "capture `{capture_id}` belongs to device `{}`, not `{device}`",
                    capture.device()
                )));
            }
            Ok(capture_id.clone())
        }
        (Some(capture_id), None) => captures
            .get(capture_id)
            .map(|_| capture_id.clone())
            .ok_or_else(|| {
                McpToolError::not_found(format!("capture `{capture_id}` was not found"))
            }),
        (None, Some(device)) => find_capture_id_by_device(captures, device).ok_or_else(|| {
            McpToolError::not_found(format!("no capture is registered for device `{device}`"))
        }),
        (None, None) => match captures.len() {
            0 => Err(McpToolError::not_found(
                "no captures are currently registered; call start_capture first",
            )),
            1 => Ok(captures
                .keys()
                .next()
                .expect("single-entry map must contain a key")
                .clone()),
            _ => Err(McpToolError::invalid_params(
                "multiple captures are registered; specify captureId or device",
            )),
        },
    }
}

fn sort_capture_statuses(captures: &mut [CaptureStatus]) {
    captures.sort_by(|left, right| {
        right
            .started_at_ms
            .cmp(&left.started_at_ms)
            .then_with(|| left.capture_id.cmp(&right.capture_id))
    });
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct CompletionSignal {
    tx: Option<watch::Sender<bool>>,
}

impl CompletionSignal {
    fn new(tx: watch::Sender<bool>) -> Self {
        Self { tx: Some(tx) }
    }
}

impl Drop for CompletionSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(true);
        }
    }
}

async fn wait_for_completion(mut completion_rx: watch::Receiver<bool>) {
    if *completion_rx.borrow() {
        return;
    }

    while completion_rx.changed().await.is_ok() {
        if *completion_rx.borrow() {
            return;
        }
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected_device(id: &str) -> ConnectedDevice {
        ConnectedDevice {
            id: id.to_owned(),
            name: format!("Device {id}"),
            description: "Test device".to_owned(),
            platform: DevicePlatform::Android,
        }
    }

    fn capture_runtime(capture_id: &str, device: &ConnectedDevice) -> CaptureRuntime {
        let (capture_control, _kill_rx, _completion_tx) =
            capture::CaptureController::test_controller();
        capture_runtime_with_control(capture_id, device, capture_control)
    }

    fn capture_runtime_with_control(
        capture_id: &str,
        device: &ConnectedDevice,
        capture_control: capture::CaptureController,
    ) -> CaptureRuntime {
        let (pump_done_tx, pump_done) = watch::channel(false);
        let pump_control = capture_control.clone();
        let pump_task = tokio::spawn(async move {
            let _pump_done = CompletionSignal::new(pump_done_tx);
            pump_control.wait_for_shutdown().await;
        });

        CaptureRuntime {
            capture_id: capture_id.to_owned(),
            shared: Arc::new(CaptureShared::new(
                device.id.clone(),
                device.name.clone(),
                device.platform,
                None,
                None,
                32,
            )),
            capture_control,
            pump_done,
            pump_task,
            shutdown_requested: false,
        }
    }

    #[tokio::test]
    async fn prepare_start_requires_restart_for_running_capture() {
        let device = connected_device("device-1");
        let mut inner = RuntimeInner::default();
        inner.captures.insert(
            "capture-1".to_owned(),
            capture_runtime("capture-1", &device),
        );

        let err = inner.prepare_start(&device, false).unwrap_err();
        assert_eq!(err.code, "conflict");
        assert!(err.message.contains("restart=true"));
    }

    #[tokio::test]
    async fn prepare_start_requests_shutdown_without_removing_capture() {
        let device = connected_device("device-1");
        let (capture_control, mut kill_rx, completion_tx) =
            capture::CaptureController::test_controller();
        let mut inner = RuntimeInner::default();
        inner.captures.insert(
            "capture-1".to_owned(),
            capture_runtime_with_control("capture-1", &device, capture_control),
        );

        let plan = inner.prepare_start(&device, true).unwrap();
        assert_eq!(plan.replaced_capture_id.as_deref(), Some("capture-1"));
        assert!(plan.shutdown.is_some());
        assert!(inner.captures.contains_key("capture-1"));
        assert_eq!(kill_rx.recv().await, Some(()));

        let snapshot = inner.captures["capture-1"].snapshot();
        assert_eq!(
            snapshot.stop_reason.as_deref(),
            Some(RESTART_CAPTURE_REASON)
        );
        assert!(snapshot.running);

        let _ = completion_tx.send(true);
    }

    #[tokio::test]
    async fn prepare_stop_requests_shutdown_without_removing_capture() {
        let device = connected_device("device-2");
        let (capture_control, mut kill_rx, completion_tx) =
            capture::CaptureController::test_controller();
        let mut inner = RuntimeInner::default();
        inner.captures.insert(
            "capture-2".to_owned(),
            capture_runtime_with_control("capture-2", &device, capture_control),
        );

        let _shutdown = inner
            .prepare_stop(&CaptureSelector::new(Some("capture-2".to_owned()), None))
            .unwrap();
        assert!(inner.captures.contains_key("capture-2"));
        assert_eq!(kill_rx.recv().await, Some(()));

        let snapshot = inner.captures["capture-2"].snapshot();
        assert_eq!(snapshot.stop_reason.as_deref(), Some(STOP_CAPTURE_REASON));
        assert!(snapshot.running);

        let _ = completion_tx.send(true);
    }

    #[tokio::test]
    async fn restart_wait_timeout_has_clear_error() {
        let device = connected_device("device-3");
        let (capture_control, _kill_rx, _completion_tx) =
            capture::CaptureController::test_controller();
        let mut capture = capture_runtime_with_control("capture-3", &device, capture_control);
        let shutdown = capture.request_shutdown(RESTART_CAPTURE_REASON);

        let err = shutdown
            .wait_with_timeout(
                Duration::from_millis(10),
                "waiting for existing capture `capture-3` on device `device-3` to stop before restart"
                    .to_owned(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "conflict");
        assert!(err.message.contains("timed out"));
        assert!(err.message.contains("before restart"));
    }
}
