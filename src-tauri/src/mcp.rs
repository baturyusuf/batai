//! MCP stdio transport for the shared Rust application kernel.

use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, Json, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    application::{
        BataiApplication, CreateGithubIssueRequest, DecisionRequest, LegacyAssignTask,
        LegacyCreateAgent,
    },
    daemon::{ClientOrigin, ControlService, DaemonClient},
    providers::process_supervisor::redact_secrets,
    runtime::meetings::CreateMeetingRequest,
};

#[derive(Clone)]
pub struct DirectorMcpServer {
    backend: McpBackend,
    tool_router: ToolRouter<Self>,
}

#[derive(Clone)]
enum McpBackend {
    Application(Arc<ControlService>),
    Daemon(Arc<tokio::sync::RwLock<DaemonClient>>),
}

impl DirectorMcpServer {
    pub fn new(application: Arc<BataiApplication>) -> Self {
        Self {
            backend: McpBackend::Application(Arc::new(ControlService::new(application))),
            tool_router: Self::tool_router(),
        }
    }

    pub fn from_daemon(client: DaemonClient) -> Self {
        Self {
            backend: McpBackend::Daemon(Arc::new(tokio::sync::RwLock::new(client))),
            tool_router: Self::tool_router(),
        }
    }

    async fn invoke<T: Serialize>(&self, method: &str, input: T) -> Result<Json<Value>, String> {
        let params = serde_json::to_value(input).map_err(|error| error.to_string())?;
        let result = match &self.backend {
            McpBackend::Application(service) => {
                service.invoke(ClientOrigin::Mcp, method, params).await
            }
            McpBackend::Daemon(client) => {
                let current = client.read().await.clone();
                match current.call_value(method, params).await {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        // Reconnect for subsequent MCP calls, but never replay the
                        // current request: its mutation outcome may be uncertain.
                        if let Ok(reconnected) = current.reconnect().await {
                            *client.write().await = reconnected;
                        }
                        Err(error)
                    }
                }
            }
        }?;
        Ok(Json(result))
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names = self
            .tool_router
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
}

#[tool_router(router = tool_router)]
impl DirectorMcpServer {
    #[tool(
        name = "batai_create_agent",
        description = "Create a task-scoped or project agent through Rust governance. Permanent or privileged agents may require an exact GOD decision."
    )]
    async fn create_agent(
        &self,
        Parameters(input): Parameters<LegacyCreateAgent>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_create_agent", input).await
    }

    #[tool(
        name = "batai_assign_task",
        description = "Create and evaluate a typed Batai task through the Rust task engine."
    )]
    async fn assign_task(
        &self,
        Parameters(input): Parameters<LegacyAssignTask>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_assign_task", input).await
    }

    #[tool(
        name = "batai_read_project_state",
        description = "Read the same secret-filtered Rust application snapshot used by the desktop and HTTP control plane."
    )]
    async fn read_project_state(&self) -> Result<Json<Value>, String> {
        self.invoke("batai_read_project_state", json!({})).await
    }

    #[tool(
        name = "batai_approve_task",
        description = "Approve a task at the Director review gate; this does not bypass GOD governance."
    )]
    async fn approve_task(
        &self,
        Parameters(input): Parameters<TaskIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_approve_task", input).await
    }

    #[tool(
        name = "batai_create_worktree",
        description = "Create or reuse the task's isolated managed Git worktree; the default branch is never used as an agent worktree."
    )]
    async fn create_worktree(
        &self,
        Parameters(input): Parameters<CreateWorktreeArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_create_worktree", input).await
    }

    #[tool(
        name = "batai_update_directives",
        description = "Replace an agent's directives through the recoverable Rust file journal and audit trail."
    )]
    async fn update_directives(
        &self,
        Parameters(input): Parameters<UpdateDirectivesArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_update_directives", input).await
    }

    #[tool(
        name = "batai_resume_agent",
        description = "Ask the Rust task engine to resume tasks waiting for this agent's resource."
    )]
    async fn resume_agent(
        &self,
        Parameters(input): Parameters<AgentIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_resume_agent", input).await
    }

    #[tool(
        name = "batai_create_github_issue",
        description = "Request GitHub issue creation through Rust delivery and AuthorityRouter. This may create a GOD decision and never bypasses merge or provider policy."
    )]
    async fn create_github_issue(
        &self,
        Parameters(input): Parameters<CreateGithubIssueRequest>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_create_github_issue", input).await
    }

    #[tool(
        name = "batai_read_director_inbox",
        description = "Read GOD messages addressed to the Director from the project authority inbox."
    )]
    async fn read_director_inbox(
        &self,
        Parameters(input): Parameters<ReadInboxArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_read_director_inbox", input).await
    }

    #[tool(
        name = "batai_acknowledge_god_message",
        description = "Acknowledge one exact GOD message using a recoverable and audited Rust mutation."
    )]
    async fn acknowledge_god_message(
        &self,
        Parameters(input): Parameters<MessageIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_acknowledge_god_message", input).await
    }

    #[tool(
        name = "batai_request_god_decision",
        description = "Create a structured GOD decision request through Rust governance. The Director cannot resolve it."
    )]
    async fn request_god_decision(
        &self,
        Parameters(input): Parameters<DecisionRequest>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_request_god_decision", input).await
    }

    #[tool(name = "batai_get_task", description = "Read one typed task by ID.")]
    async fn get_task(
        &self,
        Parameters(input): Parameters<TaskIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_get_task", input).await
    }

    #[tool(
        name = "batai_get_resources",
        description = "Read the governed intelligence resource portfolio without credentials or secret diagnostics."
    )]
    async fn get_resources(&self) -> Result<Json<Value>, String> {
        self.invoke("batai_get_resources", json!({})).await
    }

    #[tool(
        name = "batai_get_delivery",
        description = "Read the Rust GitHub delivery checkpoint for a task."
    )]
    async fn get_delivery(
        &self,
        Parameters(input): Parameters<TaskIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_get_delivery", input).await
    }

    #[tool(
        name = "batai_get_decisions",
        description = "Read governance decisions and provider approvals from the Rust governance snapshot."
    )]
    async fn get_decisions(&self) -> Result<Json<Value>, String> {
        self.invoke("batai_get_decisions", json!({})).await
    }

    #[tool(
        name = "batai_get_recovery_state",
        description = "Read unfinished or review-required recoverable operations; this tool cannot force recovery."
    )]
    async fn get_recovery_state(&self) -> Result<Json<Value>, String> {
        self.invoke("batai_get_recovery_state", json!({})).await
    }

    #[tool(
        name = "batai_create_meeting",
        description = "Create a bounded meeting through the daemon Meeting Engine. Participants, rounds, tokens, PAYG and authority remain policy-enforced."
    )]
    async fn create_meeting(
        &self,
        Parameters(input): Parameters<CreateMeetingRequest>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_create_meeting", input).await
    }

    #[tool(
        name = "batai_get_meeting",
        description = "Read one persisted meeting and its current bounded execution state."
    )]
    async fn get_meeting(
        &self,
        Parameters(input): Parameters<MeetingIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_get_meeting", input).await
    }

    #[tool(
        name = "batai_list_meetings",
        description = "List persisted meetings without provider transcripts or hidden reasoning."
    )]
    async fn list_meetings(&self) -> Result<Json<Value>, String> {
        self.invoke("batai_list_meetings", json!({})).await
    }

    #[tool(
        name = "batai_cancel_meeting",
        description = "Cancel one exact meeting and request cancellation of its live provider turns. Partial output is never promoted to a final decision."
    )]
    async fn cancel_meeting(
        &self,
        Parameters(input): Parameters<MeetingIdArgs>,
    ) -> Result<Json<Value>, String> {
        self.invoke("batai_cancel_meeting", input).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DirectorMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Use Batai's governed Rust tools for deterministic orchestration. MCP acts as DIRECTOR, not GOD, and cannot bypass approvals.",
            )
            .with_server_info(rmcp::model::Implementation::new("batai-control", env!("CARGO_PKG_VERSION")))
    }
}

pub async fn run_stdio(application: Arc<BataiApplication>) -> Result<(), String> {
    run_server(DirectorMcpServer::new(application)).await
}

pub async fn run_stdio_bridge(client: DaemonClient) -> Result<(), String> {
    run_server(DirectorMcpServer::from_daemon(client)).await
}

async fn run_server(server: DirectorMcpServer) -> Result<(), String> {
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| redact_secrets(&error.to_string()))?;
    service
        .waiting()
        .await
        .map(|_| ())
        .map_err(|error| redact_secrets(&error.to_string()))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct TaskIdArgs {
    task_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AgentIdArgs {
    agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct MeetingIdArgs {
    meeting_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct MessageIdArgs {
    message_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct CreateWorktreeArgs {
    agent_id: String,
    task_id: String,
    #[serde(default)]
    base_ref: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct UpdateDirectivesArgs {
    agent_id: String,
    content: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
struct ReadInboxArgs {
    #[serde(default)]
    pending_only: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{
        model::{
            ClientCapabilities, ClientJsonRpcMessage, Implementation, ProtocolVersion,
            RequestMetaObject, ServerJsonRpcMessage, ServerResult,
        },
        transport::{IntoTransport, Transport},
        ClientHandler,
    };

    #[derive(Clone, Default)]
    struct DiscoveryClient;

    impl ClientHandler for DiscoveryClient {}

    #[test]
    fn legacy_tool_names_are_preserved_by_the_official_sdk_router() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../specs/control-plane-contract-v1.json"))
                .expect("contract fixture");
        let required = fixture["legacyMcpTools"].as_array().expect("tool names");
        let names = DirectorMcpServer::new(test_application()).tool_names();
        for name in required {
            let name = name.as_str().expect("tool name");
            assert!(
                names.iter().any(|candidate| candidate == name),
                "missing {name}"
            );
        }
    }

    #[tokio::test]
    async fn official_sdk_preserves_initialize_compatibility() {
        assert_eq!(negotiate("2025-11-25").await, ProtocolVersion::V_2025_11_25);
        assert_eq!(negotiate("2026-07-28").await, ProtocolVersion::LATEST);
    }

    #[tokio::test]
    async fn official_sdk_exposes_the_current_discover_lifecycle() {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
        let service = DirectorMcpServer::new(test_application());
        let server = tokio::spawn(async move {
            let service = service.serve(server_transport).await.expect("serve");
            let _ = service.waiting().await;
        });
        let client = DiscoveryClient
            .serve(client_transport)
            .await
            .expect("connect client");
        let mut meta = RequestMetaObject::new();
        meta.set_protocol_version(ProtocolVersion::V_2026_07_28);
        meta.set_client_info(Implementation::new("batai-test", "1"));
        meta.set_client_capabilities(ClientCapabilities::default());
        let discovery = client.discover(meta).await.expect("discover");
        assert!(discovery
            .supported_versions
            .contains(&ProtocolVersion::V_2026_07_28));
        client.cancel().await.expect("cancel");
        server.abort();
    }

    #[tokio::test]
    async fn official_sdk_lists_and_calls_safe_legacy_tools() {
        let (mut client, server) = connected("2025-11-25").await;
        let _ = client.receive().await.expect("initialize response");
        client
            .send(message(json!({
                "jsonrpc":"2.0","method":"notifications/initialized"
            })))
            .await
            .expect("initialized");
        client
            .send(message(json!({
                "jsonrpc":"2.0","id":2,"method":"tools/list","params":{}
            })))
            .await
            .expect("tools/list");
        let listed = client.receive().await.expect("tools/list response");
        let listed = serde_json::to_value(listed).expect("serialize response");
        assert!(listed.to_string().contains("batai_read_project_state"));
        assert!(listed.to_string().contains("batai_create_meeting"));
        assert!(listed.to_string().contains("batai_get_meeting"));
        assert!(listed.to_string().contains("batai_list_meetings"));
        assert!(listed.to_string().contains("batai_cancel_meeting"));

        client
            .send(message(json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"batai_read_project_state","arguments":{}}
            })))
            .await
            .expect("tools/call");
        let called = client.receive().await.expect("tools/call response");
        let called = serde_json::to_value(called).expect("serialize response");
        assert_eq!(called["result"]["isError"], false);
        assert!(called["result"]["structuredContent"].is_object());
        drop(client);
        server.abort();
    }

    #[tokio::test]
    async fn official_sdk_rejects_unknown_tools_and_invalid_params() {
        let (mut client, server) = connected("2025-11-25").await;
        let _ = client.receive().await.expect("initialize response");
        client
            .send(message(json!({
                "jsonrpc":"2.0","method":"notifications/initialized"
            })))
            .await
            .expect("initialized");

        client
            .send(message(json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"batai_not_a_tool","arguments":{}}
            })))
            .await
            .expect("unknown tool request");
        let unknown = client.receive().await.expect("unknown tool response");
        let unknown = serde_json::to_value(unknown).expect("serialize response");
        assert!(
            unknown["error"].is_object() || unknown["result"]["isError"] == true,
            "unknown tools must produce a protocol or tool error: {unknown}"
        );

        client
            .send(message(json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"batai_approve_task","arguments":{}}
            })))
            .await
            .expect("invalid params request");
        let invalid = client.receive().await.expect("invalid params response");
        let invalid = serde_json::to_value(invalid).expect("serialize response");
        assert!(
            invalid["error"].is_object() || invalid["result"]["isError"] == true,
            "invalid parameters must produce a protocol or tool error: {invalid}"
        );
        drop(client);
        server.abort();
    }

    async fn negotiate(version: &str) -> ProtocolVersion {
        let (mut client, server) = connected(version).await;
        let response = client.receive().await.expect("initialize response");
        let ServerJsonRpcMessage::Response(response) = response else {
            panic!("expected initialize response");
        };
        let ServerResult::InitializeResult(result) = response.result else {
            panic!("expected initialize result");
        };
        drop(client);
        server.abort();
        result.protocol_version
    }

    async fn connected(
        version: &str,
    ) -> (
        impl Transport<rmcp::RoleClient>,
        tokio::task::JoinHandle<()>,
    ) {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
        let service = DirectorMcpServer::new(test_application());
        let server = tokio::spawn(async move {
            let service = service.serve(server_transport).await.expect("serve");
            let _ = service.waiting().await;
        });
        let mut client = IntoTransport::<rmcp::RoleClient, _, _>::into_transport(client_transport);
        client
            .send(message(json!({
                "jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{
                    "protocolVersion":version,
                    "capabilities":{},
                    "clientInfo":{"name":"batai-test","version":"1"}
                }
            })))
            .await
            .expect("initialize");
        (client, server)
    }

    fn message(value: Value) -> ClientJsonRpcMessage {
        serde_json::from_value(value).expect("valid client message")
    }

    fn test_application() -> Arc<BataiApplication> {
        let root = tempfile::tempdir().expect("project").keep();
        std::fs::create_dir_all(root.join(".batai/tasks")).expect("task directory");
        BataiApplication::open(root, crate::application::ApplicationMode::Mcp).expect("application")
    }
}
