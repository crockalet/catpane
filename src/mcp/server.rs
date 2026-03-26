use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use tokio::io::{
    self, AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
};

use super::protocol::{
    CallToolParams, EmptyParams, EmptyResult, INTERNAL_ERROR, ImplementationInfo, InitializeParams,
    InitializeResult, JSONRPC_VERSION, JsonRpcErrorResponse, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, ListToolsParams, ListToolsResult, METHOD_INITIALIZE, METHOD_PING,
    METHOD_TOOLS_CALL, METHOD_TOOLS_LIST, NOTIFICATION_INITIALIZED, RequestId, ServerCapabilities,
    Tool,
};
use super::tools::{McpRuntimeState, handle_tool_call, tool_definitions};

pub async fn run_stdio_server(rt: tokio::runtime::Handle) -> Result<(), String> {
    let stdin = BufReader::new(io::stdin());
    let stdout = BufWriter::new(io::stdout());
    StdioMcpServer::new(rt).run(stdin, stdout).await
}

struct StdioMcpServer {
    rt: tokio::runtime::Handle,
    state: McpRuntimeState,
    server_info: ImplementationInfo,
    tools: Vec<Tool>,
    initialize_seen: bool,
}

enum InboundMessage {
    Request(JsonRpcRequest<Value>),
    Notification(JsonRpcNotification<Value>),
}

impl StdioMcpServer {
    fn new(rt: tokio::runtime::Handle) -> Self {
        Self {
            rt,
            state: McpRuntimeState::new(),
            server_info: server_info(),
            tools: tool_definitions(),
            initialize_seen: false,
        }
    }

    async fn run<R, W>(mut self, reader: R, mut writer: W) -> Result<(), String>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut lines = reader.lines();

        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|err| format!("failed to read MCP stdin: {err}"))?
        {
            if line.trim().is_empty() {
                continue;
            }

            if let Some(response) = self.handle_line(&line).await {
                write_json_line(&mut writer, &response).await?;
            }
        }

        writer
            .flush()
            .await
            .map_err(|err| format!("failed to flush MCP stdout: {err}"))?;
        Ok(())
    }

    async fn handle_line(&mut self, line: &str) -> Option<Value> {
        match parse_inbound_message(line) {
            Ok(InboundMessage::Request(request)) => Some(self.handle_request(request).await),
            Ok(InboundMessage::Notification(notification)) => {
                self.handle_notification(notification);
                None
            }
            Err(error) => Some(serialize_error_response(error)),
        }
    }

    async fn handle_request(&mut self, request: JsonRpcRequest<Value>) -> Value {
        match self.dispatch_request(request).await {
            Ok(response) => response,
            Err(error) => serialize_error_response(error),
        }
    }

    async fn dispatch_request(
        &mut self,
        request: JsonRpcRequest<Value>,
    ) -> Result<Value, JsonRpcErrorResponse<Value>> {
        match request.method.as_str() {
            METHOD_INITIALIZE => self.handle_initialize(request),
            METHOD_PING => self.handle_ping(request),
            METHOD_TOOLS_LIST => self.handle_tools_list(request),
            METHOD_TOOLS_CALL => self.handle_tools_call(request).await,
            _ => Err(JsonRpcErrorResponse::method_not_found(
                Some(request.id.clone()),
                request.method,
            )),
        }
    }

    fn handle_initialize(
        &mut self,
        request: JsonRpcRequest<Value>,
    ) -> Result<Value, JsonRpcErrorResponse<Value>> {
        if self.initialize_seen {
            return Err(JsonRpcErrorResponse::invalid_request(
                Some(request.id.clone()),
                "Server has already been initialized",
            ));
        }

        let params: InitializeParams = parse_required_params(&request)?;
        let protocol_version = match params.negotiated_protocol_version() {
            Some(protocol_version) => protocol_version,
            None => {
                return Err(JsonRpcErrorResponse::unsupported_protocol_version(
                    Some(request.id.clone()),
                    params.protocol_version,
                ));
            }
        };

        let response = success_response(
            request.id.clone(),
            InitializeResult::new(
                self.server_info.clone(),
                ServerCapabilities::with_tools(false),
            )
            .with_protocol_version(protocol_version),
        )?;
        self.initialize_seen = true;
        Ok(response)
    }

    fn handle_ping(
        &self,
        request: JsonRpcRequest<Value>,
    ) -> Result<Value, JsonRpcErrorResponse<Value>> {
        let _: EmptyParams = parse_optional_params_or_default(&request)?;
        success_response(request.id, EmptyResult::default())
    }

    fn handle_tools_list(
        &self,
        request: JsonRpcRequest<Value>,
    ) -> Result<Value, JsonRpcErrorResponse<Value>> {
        self.ensure_initialized(&request)?;

        let params: ListToolsParams = parse_optional_params_or_default(&request)?;
        if params.cursor.is_some() {
            return Err(JsonRpcErrorResponse::invalid_params(
                Some(request.id.clone()),
                "tools/list does not support cursors",
            ));
        }

        success_response(request.id, ListToolsResult::new(self.tools.clone()))
    }

    async fn handle_tools_call(
        &self,
        request: JsonRpcRequest<Value>,
    ) -> Result<Value, JsonRpcErrorResponse<Value>> {
        self.ensure_initialized(&request)?;

        let params: CallToolParams = parse_required_params(&request)?;
        let result = handle_tool_call(&self.rt, &self.state, params).await;
        success_response(request.id, result)
    }

    fn handle_notification(&mut self, notification: JsonRpcNotification<Value>) {
        if notification.method == NOTIFICATION_INITIALIZED {}
    }

    fn ensure_initialized(
        &self,
        request: &JsonRpcRequest<Value>,
    ) -> Result<(), JsonRpcErrorResponse<Value>> {
        if self.initialize_seen {
            return Ok(());
        }

        Err(JsonRpcErrorResponse::invalid_request(
            Some(request.id.clone()),
            format!(
                "{} is unavailable before initialize completes",
                request.method
            ),
        ))
    }
}

fn server_info() -> ImplementationInfo {
    ImplementationInfo::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

fn parse_inbound_message(line: &str) -> Result<InboundMessage, JsonRpcErrorResponse<Value>> {
    let value = serde_json::from_str::<Value>(line).map_err(|err| {
        JsonRpcErrorResponse::parse_error(format!("failed to parse JSON-RPC message: {err}"))
    })?;
    let object = value.as_object().cloned().ok_or_else(|| {
        JsonRpcErrorResponse::invalid_request(None, "JSON-RPC message must be an object")
    })?;
    let request_id = extract_request_id(&object);

    match object.get("jsonrpc") {
        Some(Value::String(version)) if version == JSONRPC_VERSION => {}
        Some(_) => {
            return Err(JsonRpcErrorResponse::invalid_request(
                request_id.clone(),
                format!("jsonrpc must be \"{JSONRPC_VERSION}\""),
            ));
        }
        None => {
            return Err(JsonRpcErrorResponse::invalid_request(
                request_id.clone(),
                "Missing jsonrpc version",
            ));
        }
    }

    let has_method = object.contains_key("method");
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");

    if has_result || has_error {
        return Err(JsonRpcErrorResponse::invalid_request(
            request_id.clone(),
            "Client-to-server messages must be requests or notifications",
        ));
    }

    if !has_method {
        return Err(JsonRpcErrorResponse::invalid_request(
            request_id.clone(),
            "Missing method",
        ));
    }

    match object.get("method") {
        Some(Value::String(method)) if !method.is_empty() => {}
        _ => {
            return Err(JsonRpcErrorResponse::invalid_request(
                request_id.clone(),
                "method must be a non-empty string",
            ));
        }
    }

    if object.contains_key("id") {
        match object.get("id") {
            Some(Value::String(_)) | Some(Value::Number(_)) => {}
            Some(_) => {
                return Err(JsonRpcErrorResponse::invalid_request(
                    None,
                    "id must be a string or number when present",
                ));
            }
            None => unreachable!(),
        }

        let request = serde_json::from_value::<JsonRpcRequest<Value>>(value).map_err(|err| {
            JsonRpcErrorResponse::invalid_request(
                request_id,
                format!("Invalid request payload: {err}"),
            )
        })?;
        Ok(InboundMessage::Request(request))
    } else {
        let notification =
            serde_json::from_value::<JsonRpcNotification<Value>>(value).map_err(|err| {
                JsonRpcErrorResponse::invalid_request(
                    None,
                    format!("Invalid notification payload: {err}"),
                )
            })?;
        Ok(InboundMessage::Notification(notification))
    }
}

fn extract_request_id(object: &Map<String, Value>) -> Option<RequestId> {
    match object.get("id") {
        Some(Value::String(value)) => Some(RequestId::from(value.clone())),
        Some(Value::Number(value)) => Some(RequestId::Number(value.clone())),
        _ => None,
    }
}

fn parse_required_params<T>(
    request: &JsonRpcRequest<Value>,
) -> Result<T, JsonRpcErrorResponse<Value>>
where
    T: DeserializeOwned,
{
    request
        .deserialize_params::<T>()
        .map_err(|err| invalid_params_response(request, err))?
        .ok_or_else(|| {
            JsonRpcErrorResponse::invalid_params(
                Some(request.id.clone()),
                format!("{} requires params", request.method),
            )
        })
}

fn parse_optional_params_or_default<T>(
    request: &JsonRpcRequest<Value>,
) -> Result<T, JsonRpcErrorResponse<Value>>
where
    T: Default + DeserializeOwned,
{
    request
        .deserialize_params_or_default::<T>()
        .map_err(|err| invalid_params_response(request, err))
}

fn invalid_params_response(
    request: &JsonRpcRequest<Value>,
    err: serde_json::Error,
) -> JsonRpcErrorResponse<Value> {
    JsonRpcErrorResponse::invalid_params(
        Some(request.id.clone()),
        format!("Invalid params for {}: {err}", request.method),
    )
}

fn success_response<R>(id: RequestId, result: R) -> Result<Value, JsonRpcErrorResponse<Value>>
where
    R: Serialize,
{
    let response_id = id.clone();
    serde_json::to_value(JsonRpcResponse::new(id, result)).map_err(|err| {
        JsonRpcErrorResponse::internal_error(
            Some(response_id),
            format!("failed to serialize response: {err}"),
        )
    })
}

fn serialize_error_response(error: JsonRpcErrorResponse<Value>) -> Value {
    serde_json::to_value(error).unwrap_or_else(|_| {
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": Value::Null,
            "error": {
                "code": INTERNAL_ERROR,
                "message": "failed to serialize error response",
            }
        })
    })
}

async fn write_json_line<W>(writer: &mut W, value: &Value) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(value)
        .map_err(|err| format!("failed to serialize MCP response: {err}"))?;
    bytes.push(b'\n');

    writer
        .write_all(&bytes)
        .await
        .map_err(|err| format!("failed to write MCP response: {err}"))?;
    writer
        .flush()
        .await
        .map_err(|err| format!("failed to flush MCP response: {err}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::mcp::protocol::{
        InitializeResponse, MCP_PROTOCOL_VERSION_2024_11_05, MCP_PROTOCOL_VERSION_2025_11_25,
    };

    fn initialize_request(protocol_version: &str) -> JsonRpcRequest<Value> {
        JsonRpcRequest::with_params(
            1u32,
            METHOD_INITIALIZE,
            json!({
                "protocolVersion": protocol_version,
                "capabilities": {},
                "clientInfo": {
                    "name": "catpane-tests",
                    "version": "0.1.0"
                }
            }),
        )
    }

    #[test]
    fn handle_initialize_echoes_supported_protocol_version() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should initialize");

        for protocol_version in [
            MCP_PROTOCOL_VERSION_2025_11_25,
            MCP_PROTOCOL_VERSION_2024_11_05,
        ] {
            let mut server = StdioMcpServer::new(runtime.handle().clone());
            let response = server
                .handle_initialize(initialize_request(protocol_version))
                .expect("initialize should succeed for supported versions");
            let response = serde_json::from_value::<InitializeResponse>(response)
                .expect("initialize response should deserialize");

            assert_eq!(response.result.protocol_version, protocol_version);
        }
    }
}
