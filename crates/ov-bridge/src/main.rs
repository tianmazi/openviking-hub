mod hook;
mod mcp;

use clap::{Parser, Subcommand};
use ov_core::ov_client::OvClient;
use ov_core::policy::model::AgentId;
use ov_core::policy::store::PolicyStore;

#[derive(Parser)]
#[command(
    name = "ov-bridge",
    version,
    about = "Cross-platform zero-dependency stdio MCP Server & lifecycle Hook runner for OpenViking Desktop"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a JSON-RPC 2.0 stdio MCP server exposing policy-governed OpenViking tools.
    Mcp {
        #[arg(long, default_value = "claude-code")]
        agent: String,
    },
    /// Execute a lifecycle Hook event (session-start | user-prompt-submit | stop).
    Hook {
        event: String,
        #[arg(long, default_value = "claude-code")]
        agent: String,
    },
    /// Print remote OpenViking server status and loaded policy summary as JSON.
    Status,
    /// Drain the active profile's durable capture queue.
    Sync { #[arg(long)] profile: Option<String> },
    /// Inspect the active profile's capture queue and connectivity.
    Doctor,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Mcp { agent } => {
            let agent_id = AgentId::parse_str(&agent).unwrap_or(AgentId::ClaudeCode);
            mcp::run_stdio_server(agent_id).await?;
        }
        Commands::Hook { event, agent } => {
            let agent_id = AgentId::parse_str(&agent).unwrap_or(AgentId::ClaudeCode);
            hook::run_hook(&event, agent_id).await?;
        }
        Commands::Sync { profile } => {
            let client = OvClient::from_local_config()?;
            if profile.as_deref().is_some_and(|p| p != client.profile_key()) { return Ok(()); }
            let policy = PolicyStore::load_or_init()?;
            if policy.global.capture_enabled {
                let db = ov_core::dreaming::db::DesktopDb::open_default().map_err(anyhow::Error::msg)?;
                let count = ov_core::capture::sync_capture(&db, &client).await.map_err(anyhow::Error::msg)?;
                println!("{}", serde_json::json!({"committed_sessions":count}));
            }
        }
        Commands::Doctor | Commands::Status => {
            let client = OvClient::from_local_config()?;
            let status = client.status().await;
            let policy = PolicyStore::load_or_init().ok();
            let capture = if let Ok(db) = ov_core::dreaming::db::DesktopDb::open_default() {
                let _ = ov_core::capture::refresh_extraction_tasks(&db,&client).await;
                db.capture_health(&client.profile_key()).ok()
            } else { None };
            let out = serde_json::json!({
                "capture_queue": capture,
                "agents": ov_core::adapters::AdapterRegistry::new().detect_all(),
                "server": status,
                "capture_enabled": policy.as_ref().map(|p| p.global.capture_enabled).unwrap_or(true),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }
    Ok(())
}
