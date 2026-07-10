use std::time::Duration;

use rmcp::{
    model::{CallToolRequestParams, ErrorCode, Tool},
    service::{RunningService, ServiceError},
    transport::TokioChildProcess,
    RoleClient, ServiceExt,
};
use tokio::process::Command;

use crate::models::{McpError, ToolDefinition, ToolFunctionDef};

// ── rmcp → McpError mapping helpers ──────────────────────────────────────────

fn map_error_data(data: rmcp::model::ErrorData) -> McpError {
    match data.code {
        ErrorCode::METHOD_NOT_FOUND => McpError::ToolNotFound { message: data.message.to_string() },
        ErrorCode::INVALID_PARAMS   => McpError::InvalidParams { message: data.message.to_string() },
        other => McpError::Protocol { code: other.0, message: data.message.to_string() },
    }
}

fn map_service_error(err: ServiceError) -> McpError {
    match err {
        ServiceError::TransportClosed => McpError::ConnectionLost,
        ServiceError::TransportSend(_) => McpError::ConnectionLost,
        ServiceError::Timeout { timeout } => McpError::Timeout { elapsed: timeout },
        ServiceError::Cancelled { reason } => McpError::Cancelled { reason },
        ServiceError::UnexpectedResponse => McpError::Protocol {
            code: -1,
            message: "unexpected server response".into(),
        },
        ServiceError::McpError(data) => map_error_data(data),
        // ServiceError is #[non_exhaustive] — mandatory fallback
        _ => McpError::Protocol { code: -1, message: err.to_string() },
    }
}

fn map_init_error(err: rmcp::service::ClientInitializeError) -> McpError {
    use rmcp::service::ClientInitializeError;
    match err {
        ClientInitializeError::ConnectionClosed(msg) => McpError::InitializationFailed { message: msg },
        ClientInitializeError::TransportError { error, context } => McpError::InitializationFailed {
            message: format!("{context}: {error}"),
        },
        ClientInitializeError::JsonRpcError(data) => map_error_data(data),
        ClientInitializeError::Cancelled => McpError::Cancelled { reason: None },
        // Other variants (ExpectedInitResponse, etc.) → InitializationFailed
        _ => McpError::InitializationFailed { message: err.to_string() },
    }
}

// ── McpManager ────────────────────────────────────────────────────────────────

pub struct McpManager {
    client: Option<RunningService<RoleClient, ()>>,
    pub project_path: Option<String>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            client: None,
            project_path: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    pub fn project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    pub async fn connect(&mut self, project_path: &str) -> Result<(), McpError> {
        // Disconnect existing client if any
        if let Some(client) = self.client.take() {
            let _ = client.cancel().await;
        }

        let mut cmd = Command::new("npx");
        cmd.arg("-y")
            .arg("@modelcontextprotocol/server-filesystem")
            .arg(project_path);

        let transport = TokioChildProcess::new(cmd)
            .map_err(|e| McpError::InitializationFailed { message: format!("spawn: {e}") })?;

        let client = ()
            .serve(transport)
            .await
            .map_err(map_init_error)?;

        self.client = Some(client);
        self.project_path = Some(project_path.to_string());

        log::info!("MCP connected to project: {project_path}");
        Ok(())
    }

    pub async fn disconnect(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.cancel().await;
        }
        self.project_path = None;
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>, McpError> {
        let client = self
            .client
            .as_ref()
            .ok_or(McpError::ConnectionLost)?;

        let result = client
            .list_tools(Default::default())
            .await
            .map_err(map_service_error)?;

        Ok(result.tools)
    }

    /// Convert MCP tools to Ollama's tool definition format.
    pub async fn get_tools_for_ollama(&self) -> Result<Vec<ToolDefinition>, McpError> {
        let mcp_tools = self.list_tools().await?;

        let tools = mcp_tools
            .into_iter()
            .map(|t| ToolDefinition {
                tool_type: "function".to_string(),
                function: ToolFunctionDef {
                    name: t.name.to_string(),
                    description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                    parameters: serde_json::to_value(&t.input_schema).unwrap_or_default(),
                },
            })
            .collect();

        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, McpError> {
        let client = self
            .client
            .as_ref()
            .ok_or(McpError::ConnectionLost)?;

        let args = arguments.as_object().cloned();

        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = args;

        let result = client
            .call_tool(params)
            .await
            .map_err(map_service_error)?;

        // Extract text content from tool result
        let content: String = result
            .content
            .into_iter()
            .filter_map(|c| match c.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(content)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ErrorData;

    fn make_service_error_mcp(code: i32, msg: &str) -> ServiceError {
        let error_data = ErrorData::new(
            rmcp::model::ErrorCode(code),
            msg.to_string(),
            None,
        );
        ServiceError::McpError(error_data)
    }

    #[test]
    fn transport_closed_maps_to_connection_lost() {
        let err = map_service_error(ServiceError::TransportClosed);
        assert!(matches!(err, McpError::ConnectionLost));
    }

    #[test]
    fn timeout_maps_to_mcp_timeout_with_duration_preserved() {
        let dur = Duration::from_secs(5);
        let err = map_service_error(ServiceError::Timeout { timeout: dur });
        assert!(matches!(err, McpError::Timeout { elapsed } if elapsed == dur));
    }

    #[test]
    fn json_rpc_minus_32601_maps_to_tool_not_found() {
        let err = map_service_error(make_service_error_mcp(-32601, "not found"));
        assert!(matches!(err, McpError::ToolNotFound { .. }));
    }

    #[test]
    fn json_rpc_minus_32602_maps_to_invalid_params() {
        let err = map_service_error(make_service_error_mcp(-32602, "bad params"));
        assert!(matches!(err, McpError::InvalidParams { .. }));
    }

    #[test]
    fn cancelled_maps_to_cancelled_with_reason() {
        let err = map_service_error(ServiceError::Cancelled { reason: Some("stop".into()) });
        assert!(matches!(err, McpError::Cancelled { reason: Some(_) }));
    }

    #[test]
    fn unknown_json_rpc_code_maps_to_protocol() {
        let err = map_service_error(make_service_error_mcp(-99999, "unknown"));
        assert!(matches!(err, McpError::Protocol { .. }));
    }

    #[test]
    fn unexpected_response_maps_to_protocol() {
        let err = map_service_error(ServiceError::UnexpectedResponse);
        assert!(matches!(err, McpError::Protocol { .. }));
    }
}
