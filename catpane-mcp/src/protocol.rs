use std::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Number, Value, json};

pub const JSONRPC_VERSION: &str = "2.0";
pub const MCP_PROTOCOL_VERSION_2024_11_05: &str = "2024-11-05";
pub const MCP_PROTOCOL_VERSION_2025_11_25: &str = "2025-11-25";
pub const MCP_PROTOCOL_VERSION: &str = MCP_PROTOCOL_VERSION_2025_11_25;
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    MCP_PROTOCOL_VERSION_2025_11_25,
    MCP_PROTOCOL_VERSION_2024_11_05,
];

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_PING: &str = "ping";
pub const METHOD_TOOLS_LIST: &str = "tools/list";
pub const METHOD_TOOLS_CALL: &str = "tools/call";
pub const NOTIFICATION_INITIALIZED: &str = "notifications/initialized";
pub const NOTIFICATION_TOOLS_LIST_CHANGED: &str = "notifications/tools/list_changed";

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

pub type JsonObject = Map<String, Value>;
pub type Cursor = String;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    Number(Number),
}

impl RequestId {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            Self::Number(_) => None,
        }
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => f.write_str(value),
            Self::Number(value) => write!(f, "{value}"),
        }
    }
}

impl From<String> for RequestId {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RequestId {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<i32> for RequestId {
    fn from(value: i32) -> Self {
        Self::Number(Number::from(value))
    }
}

impl From<u32> for RequestId {
    fn from(value: u32) -> Self {
        Self::Number(Number::from(value))
    }
}

impl From<i64> for RequestId {
    fn from(value: i64) -> Self {
        Self::Number(Number::from(value))
    }
}

impl From<u64> for RequestId {
    fn from(value: u64) -> Self {
        Self::Number(Number::from(value))
    }
}

impl From<RequestId> for Value {
    fn from(value: RequestId) -> Self {
        match value {
            RequestId::String(value) => Value::String(value),
            RequestId::Number(value) => Value::Number(value),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyObject {}

pub type EmptyParams = EmptyObject;
pub type EmptyResult = EmptyObject;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub struct JsonRpcRequest<P = Value> {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
}

impl<P> JsonRpcRequest<P> {
    pub fn new(id: impl Into<RequestId>, method: impl Into<String>, params: Option<P>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }

    pub fn with_params(id: impl Into<RequestId>, method: impl Into<String>, params: P) -> Self {
        Self::new(id, method, Some(params))
    }

    pub fn without_params(id: impl Into<RequestId>, method: impl Into<String>) -> Self {
        Self::new(id, method, None)
    }
}

impl JsonRpcRequest<Value> {
    pub fn deserialize_params<T>(&self) -> serde_json::Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        self.params.clone().map(serde_json::from_value).transpose()
    }

    pub fn deserialize_params_or_default<T>(&self) -> serde_json::Result<T>
    where
        T: Default + DeserializeOwned,
    {
        match &self.params {
            Some(params) => serde_json::from_value(params.clone()),
            None => Ok(T::default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub struct JsonRpcNotification<P = Value> {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
}

impl<P> JsonRpcNotification<P> {
    pub fn new(method: impl Into<String>, params: Option<P>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            method: method.into(),
            params,
        }
    }

    pub fn with_params(method: impl Into<String>, params: P) -> Self {
        Self::new(method, Some(params))
    }

    pub fn without_params(method: impl Into<String>) -> Self {
        Self::new(method, None)
    }
}

impl JsonRpcNotification<Value> {
    pub fn deserialize_params<T>(&self) -> serde_json::Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        self.params.clone().map(serde_json::from_value).transpose()
    }

    pub fn deserialize_params_or_default<T>(&self) -> serde_json::Result<T>
    where
        T: Default + DeserializeOwned,
    {
        match &self.params {
            Some(params) => serde_json::from_value(params.clone()),
            None => Ok(T::default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "R: Deserialize<'de>"))]
pub struct JsonRpcResponse<R = Value> {
    pub jsonrpc: String,
    pub id: RequestId,
    pub result: R,
}

impl<R> JsonRpcResponse<R> {
    pub fn new(id: impl Into<RequestId>, result: R) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: id.into(),
            result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "E: Deserialize<'de>"))]
pub struct ErrorObject<E = Value> {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<E>,
}

impl<E> ErrorObject<E> {
    pub fn new(code: i32, message: impl Into<String>, data: Option<E>) -> Self {
        Self {
            code,
            message: message.into(),
            data,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "E: Deserialize<'de>"))]
pub struct JsonRpcErrorResponse<E = Value> {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    pub error: ErrorObject<E>,
}

impl<E> JsonRpcErrorResponse<E> {
    pub fn new(id: Option<RequestId>, error: ErrorObject<E>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            error,
        }
    }
}

impl JsonRpcErrorResponse<Value> {
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(None, ErrorObject::new(PARSE_ERROR, message, None))
    }

    pub fn invalid_request(id: Option<RequestId>, message: impl Into<String>) -> Self {
        Self::new(id, ErrorObject::new(INVALID_REQUEST, message, None))
    }

    pub fn method_not_found(id: Option<RequestId>, method: impl Into<String>) -> Self {
        let method = method.into();
        Self::new(
            id,
            ErrorObject::new(
                METHOD_NOT_FOUND,
                format!("Method not found: {method}"),
                None,
            ),
        )
    }

    pub fn invalid_params(id: Option<RequestId>, message: impl Into<String>) -> Self {
        Self::new(id, ErrorObject::new(INVALID_PARAMS, message, None))
    }

    pub fn internal_error(id: Option<RequestId>, message: impl Into<String>) -> Self {
        Self::new(id, ErrorObject::new(INTERNAL_ERROR, message, None))
    }

    pub fn unsupported_protocol_version(
        id: Option<RequestId>,
        requested: impl Into<String>,
    ) -> Self {
        Self::new(
            id,
            ErrorObject::new(
                INVALID_PARAMS,
                "Unsupported protocol version",
                Some(ProtocolVersionErrorData::current(requested).into()),
            ),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest<Value>),
    Notification(JsonRpcNotification<Value>),
    Response(JsonRpcResponse<Value>),
    Error(JsonRpcErrorResponse<Value>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolVersionErrorData {
    pub requested: String,
    pub supported: Vec<String>,
}

impl ProtocolVersionErrorData {
    pub fn current(requested: impl Into<String>) -> Self {
        Self {
            requested: requested.into(),
            supported: SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .map(|version| (*version).to_owned())
                .collect(),
        }
    }
}

impl From<ProtocolVersionErrorData> for Value {
    fn from(value: ProtocolVersionErrorData) -> Self {
        json!({
            "requested": value.requested,
            "supported": value.supported,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementationInfo {
    pub name: String,
    pub version: String,
}

impl ImplementationInfo {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChangedCapability {
    #[serde(default, skip_serializing_if = "is_false")]
    pub list_changed: bool,
}

impl ListChangedCapability {
    pub const fn new(list_changed: bool) -> Self {
        Self { list_changed }
    }
}

pub type RootsCapability = ListChangedCapability;
pub type PromptsCapability = ListChangedCapability;
pub type ToolsCapability = ListChangedCapability;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    #[serde(default, skip_serializing_if = "is_false")]
    pub subscribe: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub list_changed: bool,
}

impl ResourcesCapability {
    pub const fn new(subscribe: bool, list_changed: bool) -> Self {
        Self {
            subscribe,
            list_changed,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<JsonObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<EmptyObject>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<JsonObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<EmptyObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
}

impl ServerCapabilities {
    pub fn with_tools(list_changed: bool) -> Self {
        Self {
            tools: Some(ToolsCapability::new(list_changed)),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ImplementationInfo,
}

impl InitializeParams {
    pub fn new(client_info: ImplementationInfo, capabilities: ClientCapabilities) -> Self {
        Self {
            protocol_version: MCP_PROTOCOL_VERSION.to_owned(),
            capabilities,
            client_info,
        }
    }

    pub fn uses_supported_protocol_version(&self) -> bool {
        self.negotiated_protocol_version().is_some()
    }

    pub fn negotiated_protocol_version(&self) -> Option<&'static str> {
        SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .find(|version| **version == self.protocol_version)
            .copied()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ImplementationInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl InitializeResult {
    pub fn new(server_info: ImplementationInfo, capabilities: ServerCapabilities) -> Self {
        Self {
            protocol_version: MCP_PROTOCOL_VERSION.to_owned(),
            capabilities,
            server_info,
            instructions: None,
        }
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    pub fn with_protocol_version(mut self, protocol_version: impl Into<String>) -> Self {
        self.protocol_version = protocol_version.into();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Cursor>,
}

impl ListToolsParams {
    pub fn with_cursor(cursor: impl Into<Cursor>) -> Self {
        Self {
            cursor: Some(cursor.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonSchema(pub Value);

impl JsonSchema {
    pub fn new(schema: Value) -> Self {
        Self(schema)
    }

    pub fn empty_object() -> Self {
        Self::object(JsonObject::new())
    }

    pub fn object(properties: JsonObject) -> Self {
        Self(json!({
            "type": "object",
            "properties": properties,
        }))
    }

    pub fn with_required<I, S>(mut self, required: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let required = required.into_iter().map(Into::into).collect::<Vec<_>>();
        if required.is_empty() {
            return self;
        }

        if let Value::Object(schema) = &mut self.0 {
            schema.insert(
                "required".to_owned(),
                Value::Array(required.into_iter().map(Value::String).collect()),
            );
        }

        self
    }
}

impl From<Value> for JsonSchema {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

impl From<JsonObject> for JsonSchema {
    fn from(value: JsonObject) -> Self {
        Self(Value::Object(value))
    }
}

impl From<JsonSchema> for Value {
    fn from(value: JsonSchema) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: JsonSchema,
}

impl Tool {
    pub fn new(name: impl Into<String>, input_schema: impl Into<JsonSchema>) -> Self {
        Self {
            name: name.into(),
            description: None,
            input_schema: input_schema.into(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    pub tools: Vec<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<Cursor>,
}

impl ListToolsResult {
    pub fn new(tools: Vec<Tool>) -> Self {
        Self {
            tools,
            next_cursor: None,
        }
    }

    pub fn with_next_cursor(mut self, next_cursor: impl Into<Cursor>) -> Self {
        self.next_cursor = Some(next_cursor.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<JsonObject>,
}

impl CallToolParams {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: None,
        }
    }

    pub fn with_arguments(mut self, arguments: JsonObject) -> Self {
        self.arguments = Some(arguments);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolContent {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    Resource {
        resource: EmbeddedResource,
    },
}

impl ToolContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::Image {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    pub fn resource(resource: EmbeddedResource) -> Self {
        Self::Resource { resource }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddedResource {
    Text {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "mimeType")]
        mime_type: Option<String>,
        text: String,
    },
    Blob {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "mimeType")]
        mime_type: Option<String>,
        blob: String,
    },
}

impl EmbeddedResource {
    pub fn text(uri: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Text {
            uri: uri.into(),
            mime_type: None,
            text: text.into(),
        }
    }

    pub fn blob(
        uri: impl Into<String>,
        mime_type: impl Into<String>,
        blob: impl Into<String>,
    ) -> Self {
        Self::Blob {
            uri: uri.into(),
            mime_type: Some(mime_type.into()),
            blob: blob.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

impl CallToolResult {
    pub fn success<I>(content: I) -> Self
    where
        I: IntoIterator<Item = ToolContent>,
    {
        Self {
            content: content.into_iter().collect(),
            is_error: false,
        }
    }

    pub fn error<I>(content: I) -> Self
    where
        I: IntoIterator<Item = ToolContent>,
    {
        Self {
            content: content.into_iter().collect(),
            is_error: true,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::success([ToolContent::text(text)])
    }

    pub fn error_text(text: impl Into<String>) -> Self {
        Self::error([ToolContent::text(text)])
    }
}

pub type InitializeRequest = JsonRpcRequest<InitializeParams>;
pub type InitializeResponse = JsonRpcResponse<InitializeResult>;
pub type PingRequest = JsonRpcRequest<EmptyParams>;
pub type PingResponse = JsonRpcResponse<EmptyResult>;
pub type ListToolsRequest = JsonRpcRequest<ListToolsParams>;
pub type ListToolsResponse = JsonRpcResponse<ListToolsResult>;
pub type CallToolRequest = JsonRpcRequest<CallToolParams>;
pub type CallToolResponse = JsonRpcResponse<CallToolResult>;

pub fn initialized_notification() -> JsonRpcNotification<Value> {
    JsonRpcNotification::without_params(NOTIFICATION_INITIALIZED)
}

pub fn tools_list_changed_notification() -> JsonRpcNotification<Value> {
    JsonRpcNotification::without_params(NOTIFICATION_TOOLS_LIST_CHANGED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_initialize_request_payload() {
        let request = serde_json::from_value::<InitializeRequest>(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {
                    "roots": {
                        "listChanged": true
                    },
                    "sampling": {},
                    "elicitation": {
                        "form": {}
                    }
                },
                "clientInfo": {
                    "name": "catpane-tests",
                    "version": "0.1.0",
                    "title": "CatPane Tests"
                }
            }
        }))
        .expect("initialize request should deserialize");

        assert_eq!(request.method, METHOD_INITIALIZE);
        let params = request
            .params
            .expect("initialize request should include params");
        assert!(params.uses_supported_protocol_version());
        assert_eq!(params.client_info.name, "catpane-tests");
        assert_eq!(params.capabilities.roots, Some(RootsCapability::new(true)));
        assert_eq!(params.capabilities.sampling, Some(EmptyObject::default()));
        assert_eq!(
            params.negotiated_protocol_version(),
            Some(MCP_PROTOCOL_VERSION_2025_11_25)
        );
    }

    #[test]
    fn negotiate_supported_protocol_versions() {
        for version in [
            MCP_PROTOCOL_VERSION_2025_11_25,
            MCP_PROTOCOL_VERSION_2024_11_05,
        ] {
            let params = InitializeParams {
                protocol_version: version.to_owned(),
                capabilities: ClientCapabilities::default(),
                client_info: ImplementationInfo::new("catpane-tests", "0.1.0"),
            };

            assert_eq!(params.negotiated_protocol_version(), Some(version));
            assert!(params.uses_supported_protocol_version());
        }

        let unsupported = InitializeParams {
            protocol_version: "legacy".to_owned(),
            capabilities: ClientCapabilities::default(),
            client_info: ImplementationInfo::new("catpane-tests", "0.1.0"),
        };

        assert_eq!(unsupported.negotiated_protocol_version(), None);
        assert!(!unsupported.uses_supported_protocol_version());
    }

    #[test]
    fn serialize_tool_call_error_result() {
        let response = CallToolResponse::new(
            7,
            CallToolResult::error([
                ToolContent::text("failed"),
                ToolContent::resource(EmbeddedResource::text(
                    "resource://catpane/logs",
                    "extra context",
                )),
            ]),
        );

        let value = serde_json::to_value(response).expect("tool call response should serialize");

        assert_eq!(
            value,
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": "failed"
                        },
                        {
                            "type": "resource",
                            "resource": {
                                "uri": "resource://catpane/logs",
                                "text": "extra context"
                            }
                        }
                    ],
                    "isError": true
                }
            })
        );
    }

    #[test]
    fn unsupported_protocol_error_includes_supported_versions() {
        let error = JsonRpcErrorResponse::unsupported_protocol_version(Some(1.into()), "legacy");
        let value = serde_json::to_value(error).expect("error response should serialize");

        assert_eq!(
            value,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32602,
                    "message": "Unsupported protocol version",
                    "data": {
                        "requested": "legacy",
                        "supported": ["2025-11-25", "2024-11-05"]
                    }
                }
            })
        );
    }

    #[test]
    fn list_tools_result_omits_next_cursor_when_absent() {
        let result = ListToolsResult::new(vec![
            Tool::new("get_selected_logs", JsonSchema::empty_object())
                .with_description("Return selected CatPane log lines"),
        ]);

        let value = serde_json::to_value(result).expect("tools list result should serialize");
        assert_eq!(
            value,
            json!({
                "tools": [
                    {
                        "name": "get_selected_logs",
                        "description": "Return selected CatPane log lines",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    }
                ]
            })
        );
    }
}
