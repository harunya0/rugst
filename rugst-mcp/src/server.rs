use std::sync::Arc;

use rugst::Rugst;
use serde::Deserialize;

use schemars::JsonSchema;

use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{ServerCapabilities, ServerInfo},
    tool,
    tool_handler,
    tool_router,
    ServerHandler,
};

#[derive(Clone)]
pub struct RugstServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    rugst: Arc<Rugst>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RememberRequest {
    pub channel_id: String,
    pub author_id: String,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchRequest {
    pub channel_id: String,
    pub role: String,
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryRequest {
    pub channel_id: String,
    pub limit: i64,
}

#[tool_router]
impl RugstServer {
    pub fn new() -> anyhow::Result<Self> {
    eprintln!("Creating Rugst...");
    let db_path =
        std::env::var("RUGST_DB_PATH")
            .unwrap_or_else(|_| "memory.db".to_string());

    let rugst = Rugst::new(&db_path)?;

    eprintln!(
        "Current directory: {:?}",
        std::env::current_dir()?
    );

    eprintln!(
        "Memory DB: {:?}",
        std::fs::canonicalize("memory.db")
    );

    eprintln!("Rugst created.");

    Ok(Self {
        tool_router: Self::tool_router(),
        rugst: Arc::new(rugst),
    })
}

    #[tool(description = "Test the Rugst MCP server")]
    fn ping(&self) -> String {
        "pong".to_string()
    }

    #[tool(description = "Store a memory in Rugst")]
    async fn remember(
        &self,
        Parameters(request): Parameters<RememberRequest>,
    ) -> String {
        eprintln!("remember called");

        match self.rugst.remember(
            &request.channel_id,
            &request.author_id,
            &request.role,
            &request.content,
        ) {
            Ok(_) => {
                eprintln!("remember success");
                eprintln!(
                "remember: channel_id={}, author_id={}, role={}, content={}",
                request.channel_id,
                request.author_id,
                request.role,
                request.content
            );
                "Memory stored successfully.".to_string()
            }
            Err(e) => {
                eprintln!("remember error: {e:?}");
                format!("Failed to store memory: {e}")
            }
        }
    }

    #[tool(description = "Search memories in Rugst using semantic similarity")]
    async fn search(
        &self,
        Parameters(request): Parameters<SearchRequest>,
    ) -> String {
        let options = rugst::SearchOptions::default();

        match self.rugst.search(
            &request.channel_id,
            &request.role,
            &request.query,
            &options,
        ) {
            Ok(results) => {
                if results.is_empty() {
                    return "No memories found.".to_string();
                }
                eprintln!("search results: {}", results.len());
                eprintln!(
                    "search: channel_id={}, role={}, query={}",
                    request.channel_id,
                    request.role,
                    request.query
                );

                results
                    .into_iter()
                    .map(|r| format!("[{:.4}] {}", r.score, r.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Err(e) => format!("Search failed: {e}"),
        }
    }

    #[tool(description = "Get recent conversation history from Rugst")]
    async fn get_recent_history(
        &self,
        Parameters(request): Parameters<HistoryRequest>,
    ) -> String {
        match self.rugst.get_recent_history(
            &request.channel_id,
            request.limit,
        ) {
            Ok(results) => {
                if results.is_empty() {
                    return "No conversation history found.".to_string();
                }

                results
                    .into_iter()
                    .map(|item| format!("[{}] {}", item.0, item.1))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Err(e) => format!("Failed to get history: {e}"),
        }
    }
}

#[tool_handler]
impl ServerHandler for RugstServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();

        info.instructions = Some(
            "Rugst local semantic memory server".into(),
        );

        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .build();

        info
    }
}