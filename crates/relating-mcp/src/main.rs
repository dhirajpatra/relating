use relating_core::ProjectId;
use relating_graph::RelateBackend;
use relating_mcp::protocol::McpState;
use relating_mcp::stdio::run_stdio;
use relating_metadata::init_db;
use std::path::PathBuf;
use std::sync::Arc;

fn usage() -> ! {
    eprintln!("relating-mcp — Relating MCP server (read-only project tools)");
    eprintln!();
    eprintln!("Usage:");
    eprintln!(
        "  relating-mcp --stdio --project <PROJECT_ID> [--data-dir DIR] [--database-url URL]"
    );
    eprintln!();
    eprintln!("Transports:");
    eprintln!("  stdio   local agents on this machine (this binary)");
    eprintln!("  HTTP    served by relating-server at POST /mcp (bearer token + project header)");
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    // `.env` fills in unset variables only; real env always wins.
    relating_core::load_dotenv();
    let mut args = std::env::args().skip(1).peekable();
    let mut stdio = false;
    let mut project: Option<String> = None;
    let mut data_dir: Option<String> = None;
    let mut database_url: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stdio" => stdio = true,
            "--project" => project = args.next(),
            "--data-dir" => data_dir = args.next(),
            "--database-url" => database_url = args.next(),
            "-h" | "--help" => usage(),
            _ => {
                eprintln!("Unknown argument: {}", arg);
                usage();
            }
        }
    }

    if !stdio {
        eprintln!("Missing required flag: --stdio");
        usage();
    }
    let project = match project {
        Some(p) => p,
        None => {
            eprintln!("Missing required flag: --project <PROJECT_ID>");
            usage();
        }
    };
    let project_id = ProjectId::parse(&project).unwrap_or_else(|e| {
        eprintln!("Invalid project ID '{}': {}", project, e);
        std::process::exit(2);
    });

    // Logs to stderr: stdout carries only JSON-RPC messages.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            std::env::var("RELATING_LOG_LEVEL").unwrap_or_else(|_| "warn".to_string()),
        ))
        .init();

    let data_dir = PathBuf::from(data_dir.unwrap_or_else(|| {
        std::env::var("RELATING_DATA_DIR").unwrap_or_else(|_| "./data".to_string())
    }));
    let db_url = database_url.unwrap_or_else(|| {
        std::env::var("RELATING_DATABASE_URL")
            .unwrap_or_else(|_| format!("sqlite://{}/relating_metadata.db", data_dir.display()))
    });

    let pool = init_db(&db_url).await.unwrap_or_else(|e| {
        eprintln!("Failed to open metadata database: {}", e);
        std::process::exit(1);
    });

    let state = McpState {
        pool,
        graph: Arc::new(RelateBackend::new()),
        data_dir,
        mcp_token: None,
    };

    if let Err(e) = run_stdio(state, project_id).await {
        eprintln!("MCP server error: {}", e);
        std::process::exit(1);
    }
}
