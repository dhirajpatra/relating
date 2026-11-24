use relating_cli::checks::{
    check_aura_compat, check_conformance_gate, check_index_health, check_integrity, CheckOutcome,
    Context,
};
use relating_cli::error::CliError;
use relating_cli::eval_runner::run_eval_questions;
use relating_cli::output::{render_validate, OutputFormat, ValidateReport};
use relating_core::{AuraConnection, AuraConnectionId, ProjectId, RetrievalStrategy};
use relating_graph::GraphBackend;
use relating_metadata::{get_aura_connection_resolving_env, init_db};
use std::collections::HashMap;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!("relating — Relating project CLI (validation, tests, schema-as-code)");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  relating [--data-dir DIR] [--database-url URL] [--format text|json] <command>");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  validate --project ID [--all] [--connection CID] [--eval-questions FILE]");
    eprintln!("  aura analyze --project ID [--connection CID]");
    eprintln!("  aura migrate --project ID --connection CID [--yes] [--report FILE] [--aura-tier T] [--api-url URL]");
    eprintln!("  portability scan --dir DIR [--emit text|json|sarif] [--out FILE]");
    eprintln!("  test graph --project ID [--revision N]");
    eprintln!("  schema export --project ID --out DIR");
    eprintln!("  schema diff --project ID --file schema.json");
    eprintln!("  eval run --project ID --questions FILE [--strategy S] [--min-grounded F]");
    eprintln!("  shadow run --project ID [--connection CID] --queries FILE");
    eprintln!("  debug trace --project ID --question TEXT [--strategy S] [--no-ai]");
    eprintln!("  synth generate --project ID --out FILE [--scale F] [--seed N] [--edge-cases a,b] [--materialize]");
    eprintln!("  advisor analyze --project ID --queries FILE [--connection CID] [--measure] [--fail-on-warning]");
    eprintln!();
    eprintln!("  --connection accepts a stored connection ID or 'env'. Without it,");
    eprintln!("  AURA_URI + AURA_PASSWORD from .env are used when available.");
    eprintln!("  demo seed [--name NAME]   (create a seeded demo project; prints its ID)");
    eprintln!(
        "  secrets set --name NAME --from-file PATH   (OS keychain; needs --features os-keychain)"
    );
    eprintln!("  secrets check --name NAME   (report SET/NOT SET without printing the value)");
    eprintln!("  secrets delete --name NAME   (OS keychain; needs --features os-keychain)");
    eprintln!("  db check-postgres --url URL   (read-only; needs --features postgres)");
    eprintln!("  db init-postgres --url URL   (idempotent provision; needs --features postgres)");
    eprintln!();
    eprintln!("Exit codes: 0 pass, 1 gate/test failure, 2 usage or config error.");
    std::process::exit(2);
}

struct GlobalOpts {
    data_dir: PathBuf,
    database_url: String,
    format: OutputFormat,
}

fn parse_global(args: &mut Vec<String>) -> Result<(GlobalOpts, Vec<String>), CliError> {
    let mut data_dir: Option<String> = None;
    let mut database_url: Option<String> = None;
    let mut format = OutputFormat::Text;
    let mut rest = Vec::new();
    let mut iter = args.drain(..).peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--data-dir" => {
                data_dir = Some(
                    iter.next()
                        .ok_or_else(|| CliError::Usage("--data-dir needs a value".to_string()))?,
                );
            }
            "--database-url" => {
                database_url =
                    Some(iter.next().ok_or_else(|| {
                        CliError::Usage("--database-url needs a value".to_string())
                    })?);
            }
            "--format" => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::Usage("--format needs a value".to_string()))?;
                format = OutputFormat::parse(&value)
                    .ok_or_else(|| CliError::Usage("format must be text or json".to_string()))?;
            }
            "-h" | "--help" => usage(),
            _ => rest.push(arg),
        }
    }
    let data_dir = PathBuf::from(data_dir.unwrap_or_else(|| {
        std::env::var("RELATING_DATA_DIR").unwrap_or_else(|_| "./data".to_string())
    }));
    let database_url = database_url.unwrap_or_else(|| {
        std::env::var("RELATING_DATABASE_URL")
            .unwrap_or_else(|_| format!("sqlite://{}/relating_metadata.db", data_dir.display()))
    });
    Ok((
        GlobalOpts {
            data_dir,
            database_url,
            format,
        },
        rest,
    ))
}

fn parse_flags(rest: &[String]) -> Result<(Vec<String>, HashMap<String, String>), CliError> {
    let mut positionals = Vec::new();
    let mut flags = HashMap::new();
    let mut iter = rest.iter().peekable();
    while let Some(arg) = iter.next() {
        if let Some(name) = arg.strip_prefix("--") {
            if name == "all"
                || name == "no-ai"
                || name == "materialize"
                || name == "measure"
                || name == "fail-on-warning"
                || name == "yes"
            {
                flags.insert(name.to_string(), "true".to_string());
                continue;
            }
            let value = iter
                .next()
                .ok_or_else(|| CliError::Usage(format!("--{} needs a value", name)))?;
            flags.insert(name.to_string(), value.clone());
        } else {
            positionals.push(arg.clone());
        }
    }
    Ok((positionals, flags))
}

fn require_flag(flags: &HashMap<String, String>, name: &str) -> Result<String, CliError> {
    flags
        .get(name)
        .cloned()
        .ok_or_else(|| CliError::Usage(format!("Missing required --{}", name)))
}

fn parse_project_id(value: &str) -> Result<ProjectId, CliError> {
    ProjectId::parse(value).map_err(|e| CliError::Usage(format!("Invalid project ID: {}", e)))
}

/// Resolve an Aura connection for commands that can target Aura.
///
/// An explicit `--connection` wins: the value `env` selects the `AURA_URI`
/// connection from the environment, anything else must be a stored id (the
/// env connection's stable id also matches). Without the flag, the env
/// connection is used when available, otherwise `None` so callers can skip
/// or fail with a helpful message.
async fn resolve_connection(
    ctx: &Context,
    flags: &HashMap<String, String>,
) -> Result<Option<AuraConnection>, CliError> {
    match flags.get("connection") {
        Some(raw) if raw.trim().eq_ignore_ascii_case("env") => {
            match relating_core::aura_connection_from_env(&ctx.project_id) {
                Some(conn) => {
                    eprintln!(
                        "Using Aura connection from environment: '{}' ({})",
                        conn.name, conn.uri
                    );
                    Ok(Some(conn))
                }
                None => {
                    let status = relating_core::aura_env_status();
                    Err(CliError::Usage(format!(
                        "--connection env requested but unavailable: {}",
                        status.reason
                    )))
                }
            }
        }
        Some(conn_id_str) => {
            let conn_id = AuraConnectionId::parse(conn_id_str)
                .map_err(|e| CliError::Usage(format!("Invalid connection ID: {}", e)))?;
            get_aura_connection_resolving_env(&ctx.pool, &ctx.project_id, &conn_id)
                .await?
                .map(Some)
                .ok_or_else(|| CliError::Usage("Aura connection not found".to_string()))
        }
        None => {
            if let Some(conn) = relating_core::aura_connection_from_env(&ctx.project_id) {
                eprintln!(
                    "Using Aura connection from environment: '{}' ({})",
                    conn.name, conn.uri
                );
                Ok(Some(conn))
            } else {
                Ok(None)
            }
        }
    }
}

fn missing_connection_error() -> CliError {
    CliError::Usage(
        "Missing --connection and no Aura connection in the environment (set AURA_URI + AURA_PASSWORD, or pass --connection CID)".to_string(),
    )
}

async fn open_context(opts: &GlobalOpts, project_id: &ProjectId) -> Result<Context, CliError> {
    let pool = init_db(&opts.database_url)
        .await
        .map_err(|e| CliError::Project(format!("Cannot open metadata database: {}", e)))?;
    Context::load(pool, opts.data_dir.clone(), project_id).await
}

async fn cmd_validate(opts: &GlobalOpts, flags: &HashMap<String, String>) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let ctx = open_context(opts, &project_id).await?;
    let run_all = flags.contains_key("all");

    let mut gates = Vec::new();
    gates.push(check_conformance_gate(&ctx, "schema-conformance").await?);
    gates.push(check_integrity(&ctx).await?);
    gates.push(check_index_health(&ctx).await?);

    if run_all {
        // Aura compatibility when a connection resolves, else an honest skip.
        match resolve_connection(&ctx, flags).await? {
            Some(conn) => {
                gates.push(check_aura_compat(&ctx, Some(&conn)).await?);
            }
            None => {
                let mut skipped = CheckOutcome::pass("aura-compat");
                skipped.warnings.push(
                    "Skipped: pass --connection or set AURA_URI + AURA_PASSWORD to include Aura analysis"
                        .to_string(),
                );
                gates.push(skipped);
            }
        }

        // Retrieval evaluation when a question file is supplied.
        match flags.get("eval-questions") {
            Some(path) => {
                let content = std::fs::read_to_string(path)
                    .map_err(|e| CliError::Usage(format!("Cannot read questions file: {}", e)))?;
                let questions: Vec<String> = content
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                if questions.is_empty() {
                    return Err(CliError::Usage(
                        "Questions file has no usable questions".to_string(),
                    ));
                }
                let provider = relating_ai::create_ai_provider(&ctx.project.ai_config)?;
                let report = run_eval_questions(
                    &ctx.pool,
                    &provider,
                    &ctx.project,
                    &questions,
                    RetrievalStrategy::Hybrid,
                )
                .await?;
                let mut gate = if report.metrics.grounded_rate >= 0.5 {
                    CheckOutcome::pass("retrieval-eval")
                } else {
                    CheckOutcome::fail(
                        "retrieval-eval",
                        vec![format!(
                            "Grounded rate {:.2} is below 0.50 over {} question(s)",
                            report.metrics.grounded_rate, report.metrics.question_count
                        )],
                    )
                };
                gate.warnings.push(format!(
                    "eval: {}/{} grounded, precision {:.2}, prompt {}",
                    report.metrics.grounded_count,
                    report.metrics.question_count,
                    report.metrics.mean_citation_precision,
                    report.prompt_version
                ));
                gates.push(gate);
            }
            None => {
                let mut skipped = CheckOutcome::pass("retrieval-eval");
                skipped.warnings.push(
                    "Skipped: pass --eval-questions to include retrieval evaluation".to_string(),
                );
                gates.push(skipped);
            }
        }
    }

    let report = ValidateReport {
        passed: gates.iter().all(|g| g.passed),
        gates,
    };
    println!("{}", render_validate(&report, opts.format));
    Ok(if report.passed { 0 } else { 1 })
}

async fn cmd_aura_analyze(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let ctx = open_context(opts, &project_id).await?;
    let conn = resolve_connection(&ctx, flags)
        .await?
        .ok_or_else(missing_connection_error)?;

    let gate = check_aura_compat(&ctx, Some(&conn)).await?;
    let report = ValidateReport {
        passed: gate.passed,
        gates: vec![gate],
    };
    println!("{}", render_validate(&report, opts.format));
    Ok(if report.passed { 0 } else { 1 })
}

/// Thin HTTP client for commands that drive the running server API.
/// Sends `Authorization: Bearer $RELATING_API_TOKEN` when the token is set.
struct ApiClient {
    base: String,
    client: reqwest::Client,
    token: Option<String>,
}

impl ApiClient {
    fn from_flags(flags: &HashMap<String, String>) -> Result<Self, CliError> {
        let base = match flags.get("api-url") {
            Some(url) => url.trim_end_matches('/').to_string(),
            None => {
                let port = std::env::var("RELATING_PORT").unwrap_or_else(|_| "8080".to_string());
                format!("http://127.0.0.1:{}/api/v1", port)
            }
        };
        let token = std::env::var("RELATING_API_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| CliError::Http(format!("Cannot build HTTP client: {}", e)))?;
        Ok(Self {
            base,
            client,
            token,
        })
    }

    fn authed(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, CliError> {
        let url = format!("{}{}", self.base, path);
        let res = self
            .authed(self.client.post(&url).json(body))
            .send()
            .await
            .map_err(|e| CliError::Http(format!("POST {} failed: {}", url, e)))?;
        let status = res.status();
        let value: serde_json::Value = res
            .json()
            .await
            .map_err(|e| CliError::Http(format!("POST {} returned unreadable JSON: {}", url, e)))?;
        if !status.is_success() {
            let detail = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(CliError::Http(format!(
                "POST {} failed ({}): {}",
                path, status, detail
            )));
        }
        Ok(value)
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value, CliError> {
        let url = format!("{}{}", self.base, path);
        let res = self
            .authed(self.client.get(&url))
            .send()
            .await
            .map_err(|e| CliError::Http(format!("GET {} failed: {}", url, e)))?;
        let status = res.status();
        let value: serde_json::Value = res
            .json()
            .await
            .map_err(|e| CliError::Http(format!("GET {} returned unreadable JSON: {}", url, e)))?;
        if !status.is_success() {
            let detail = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(CliError::Http(format!(
                "GET {} failed ({}): {}",
                path, status, detail
            )));
        }
        Ok(value)
    }
}

fn confirm_interactive(prompt: &str) -> bool {
    use std::io::{self, IsTerminal, Write};
    // Fail closed when stdin is not a terminal (piped CI input refuses).
    if !io::stdin().is_terminal() {
        return false;
    }
    eprint!("{} [y/N] ", prompt);
    let _ = io::stderr().flush();
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(_) => matches!(line.trim().to_lowercase().as_str(), "y" | "yes"),
        Err(_) => false,
    }
}

/// Full approval-gated migration over the server API:
/// test -> analyze -> plan -> dry-run -> approve -> execute -> verify ->
/// report. Prints the plan summary and requires `--yes` or an interactive
/// yes before anything writes to Aura. Exit 1 on blockers/verify failure.
async fn cmd_aura_migrate(
    _opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project = require_flag(flags, "project")?;
    parse_project_id(&project)?;
    let connection = require_flag(flags, "connection")?;
    let auto_yes = flags.contains_key("yes");
    let api = ApiClient::from_flags(flags)?;

    println!("--- testing connection");
    let test = api
        .post(
            &format!("/projects/{}/connections/aura/{}/test", project, connection),
            &serde_json::json!({}),
        )
        .await?;
    if test.get("reachable").and_then(|v| v.as_bool()) != Some(true) {
        return Err(CliError::Http(format!("Aura test failed: {}", test)));
    }
    println!(
        "Reachable: {}",
        test.get("message").and_then(|v| v.as_str()).unwrap_or("ok")
    );

    println!("--- analyzing migration");
    let mut analyze_body = serde_json::json!({"connection_id": connection});
    if let Some(tier) = flags.get("aura-tier") {
        analyze_body["aura_tier"] = serde_json::Value::String(tier.clone());
    }
    let analysis = api
        .post(
            &format!("/projects/{}/migrations/analyze", project),
            &analyze_body,
        )
        .await?;
    println!(
        "Target reachable: {} agent: {}",
        analysis
            .get("target_reachable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        analysis
            .get("target_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
    );

    println!("--- planning migration");
    let mut plan_body = serde_json::json!({"connection_id": connection});
    if let Some(tier) = flags.get("aura-tier") {
        plan_body["aura_tier"] = serde_json::Value::String(tier.clone());
    }
    let plan = api
        .post(
            &format!("/projects/{}/migrations/plan", project),
            &plan_body,
        )
        .await?;
    let mid = plan
        .get("migration")
        .and_then(|m| m.get("id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| CliError::Http("Plan response has no migration id".to_string()))?
        .to_string();
    println!("Migration: {}", mid);

    println!("--- dry-run (must report zero remote writes)");
    let dry = api
        .post(
            &format!("/projects/{}/migrations/{}/dry-run", project, mid),
            &serde_json::json!({}),
        )
        .await?;
    let blockers = dry
        .get("blockers")
        .and_then(|b| b.as_array())
        .cloned()
        .unwrap_or_default();
    println!(
        "remote_writes={} blockers={}",
        dry.get("remote_writes")
            .and_then(|v| v.as_u64())
            .unwrap_or(u64::MAX),
        blockers.len()
    );
    if !blockers.is_empty() {
        eprintln!("Dry-run blockers must be resolved before approve:");
        for blocker in &blockers {
            eprintln!("  - {}", blocker.as_str().unwrap_or("?"));
        }
        return Ok(1);
    }

    let dry_plan = dry.get("plan").cloned().unwrap_or(serde_json::Value::Null);
    println!(
        "Plan summary: {} nodes, {} relationships in {} batches (mode: {}, destructive: {})",
        dry_plan
            .get("total_nodes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        dry_plan
            .get("total_relationships")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        dry_plan
            .get("total_batches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        dry_plan
            .get("write_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("?"),
        dry_plan
            .get("destructive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
    );
    if let Some(semantics) = dry_plan.get("write_semantics").and_then(|v| v.as_str()) {
        println!("Semantics: {}", semantics);
    }
    for warning in dry_plan
        .get("warnings")
        .and_then(|w| w.as_array())
        .cloned()
        .unwrap_or_default()
    {
        println!("Plan warning: {}", warning.as_str().unwrap_or("?"));
    }
    if auto_yes {
        println!("Approve this migration (writes to Aura)? [auto-yes]");
    } else if !confirm_interactive("Approve this migration (writes to Aura)?") {
        eprintln!("Not approved: re-run with --yes or answer 'yes' at the prompt.");
        return Ok(1);
    }

    println!("--- approving");
    api.post(
        &format!("/projects/{}/migrations/{}/approve", project, mid),
        &serde_json::json!({"approved": true}),
    )
    .await?;

    println!("--- executing (writes to Aura)");
    let exe = api
        .post(
            &format!("/projects/{}/migrations/{}/execute", project, mid),
            &serde_json::json!({}),
        )
        .await?;
    println!("Executed: {}", exe.get("progress").unwrap_or(&exe));

    println!("--- verifying");
    let ver = api
        .post(
            &format!("/projects/{}/migrations/{}/verify", project, mid),
            &serde_json::json!({}),
        )
        .await?;
    let passed = ver.get("passed").and_then(|v| v.as_bool()).unwrap_or(false);
    println!("Verify passed: {}", passed);
    if !passed {
        for detail in ver
            .get("details")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default()
        {
            eprintln!("Verify detail: {}", detail.as_str().unwrap_or("?"));
        }
        eprintln!("Verification reported passed=false; migration NOT trusted.");
        return Ok(1);
    }

    println!("--- fetching audit report");
    let report = api
        .get(&format!("/projects/{}/migrations/{}/report", project, mid))
        .await?;
    if let Some(path) = flags.get("report") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&report).unwrap_or_default(),
        )
        .map_err(|e| CliError::Check(format!("Cannot write '{}': {}", path, e)))?;
        println!("Report written: {}", path);
    }

    println!(
        "DONE: project {} migrated to Aura (migration {})",
        project, mid
    );
    Ok(0)
}

/// Scan a repository for Cypher portability risks. Emits text (default),
/// json, or sarif; with --out, writes the full document to a file.
/// Exit 1 when breaking (needs-rewrite) findings exist.
async fn cmd_portability_scan(
    _opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let dir = flags.get("dir").cloned().unwrap_or_else(|| ".".to_string());
    let emit = flags
        .get("emit")
        .cloned()
        .unwrap_or_else(|| "text".to_string());
    let emit = emit.to_ascii_lowercase();
    if !["text", "json", "sarif"].contains(&emit.as_str()) {
        return Err(CliError::Usage(
            "Unknown --emit (expected text, json, or sarif)".to_string(),
        ));
    }

    let scan = relating_portability::scan_repository(std::path::Path::new(&dir))?;
    let report = relating_portability::build_report(&scan);

    let document = match emit.as_str() {
        "json" => serde_json::to_string_pretty(&report)
            .map_err(|e| CliError::Check(format!("Cannot render report: {}", e)))?,
        "sarif" => serde_json::to_string_pretty(&relating_portability::to_sarif(&report))
            .map_err(|e| CliError::Check(format!("Cannot render SARIF: {}", e)))?,
        _ => {
            let mut out = format!(
                "Portability: {} files, {} queries ({} portable, {} transform, {} rewrite, {} review)\n",
                report.summary.files_scanned,
                report.summary.queries_total,
                report.summary.portable,
                report.summary.transform,
                report.summary.needs_rewrite,
                report.summary.needs_review,
            );
            for finding in &report.findings {
                if matches!(finding.verdict, relating_portability::PortVerdict::Portable) {
                    continue;
                }
                out.push_str(&format!(
                    "[{:?}] {}:{} {} — {}\n",
                    finding.verdict, finding.file, finding.line, finding.rule_id, finding.message
                ));
            }
            out
        }
    };

    if let Some(path) = flags.get("out") {
        std::fs::write(path, &document)
            .map_err(|e| CliError::Check(format!("Cannot write '{}': {}", path, e)))?;
        println!("Wrote {} report to {}", emit, path);
    } else {
        println!("{}", document);
    }

    if report.summary.needs_rewrite > 0 {
        eprintln!(
            "Portability gate: {} breaking finding(s) need a rewrite",
            report.summary.needs_rewrite
        );
        return Ok(1);
    }
    Ok(0)
}

async fn cmd_test_graph(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let ctx = open_context(opts, &project_id).await?;

    // Optional: pin to a specific revision instead of latest.
    if let Some(version_str) = flags.get("revision") {
        let version: u32 = version_str
            .parse()
            .map_err(|_| CliError::Usage("Invalid --revision (expected a number)".to_string()))?;
        let revision =
            relating_metadata::get_schema_revision_by_version(&ctx.pool, &ctx.project_id, version)
                .await?
                .ok_or_else(|| {
                    CliError::Usage(format!("Schema revision v{} not found", version))
                })?;
        let document: relating_schema::SchemaDocument =
            serde_json::from_value(revision.document)
                .map_err(|e| CliError::Check(format!("Stored revision is corrupt: {}", e)))?;
        let report = relating_schema::check_conformance(
            ctx.graph.as_ref(),
            &ctx.graph_path,
            &document,
            200,
        )?;
        let gate = if report.passed {
            CheckOutcome::pass("graph-contracts")
        } else {
            CheckOutcome::fail(
                "graph-contracts",
                report
                    .checks
                    .into_iter()
                    .filter(|c| !c.passed)
                    .map(|c| format!("{}: {}", c.subject, c.detail))
                    .collect(),
            )
        };
        let validate = ValidateReport {
            passed: gate.passed,
            gates: vec![gate],
        };
        println!("{}", render_validate(&validate, opts.format));
        return Ok(if validate.passed { 0 } else { 1 });
    }

    let gate = check_conformance_gate(&ctx, "graph-contracts").await?;
    let report = ValidateReport {
        passed: gate.passed,
        gates: vec![gate],
    };
    println!("{}", render_validate(&report, opts.format));
    Ok(if report.passed { 0 } else { 1 })
}

async fn cmd_schema_export(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let out = require_flag(flags, "out")?;
    let ctx = open_context(opts, &project_id).await?;

    let document = relating_schema::build_live_document(
        &ctx.pool,
        ctx.graph.as_ref(),
        &ctx.project_id,
        &ctx.graph_path,
        Some(ctx.project.name.clone()),
    )
    .await?;
    let files = relating_schema::export_files(&document)?;
    let out_dir = PathBuf::from(&out);
    for (rel_path, content) in &files {
        let dest = out_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CliError::Check(format!("Cannot create '{}': {}", parent.display(), e))
            })?;
        }
        std::fs::write(&dest, content)
            .map_err(|e| CliError::Check(format!("Cannot write '{}': {}", dest.display(), e)))?;
    }
    println!("Exported {} schema-as-code files to {}", files.len(), out);
    Ok(0)
}

async fn cmd_schema_diff(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let file = require_flag(flags, "file")?;
    let ctx = open_context(opts, &project_id).await?;

    let content = std::fs::read_to_string(&file)
        .map_err(|e| CliError::Usage(format!("Cannot read '{}': {}", file, e)))?;
    let document: relating_schema::SchemaDocument =
        serde_json::from_str(&content).map_err(|e| {
            CliError::Usage(format!("'{}' is not a valid schema document: {}", file, e))
        })?;
    document.validate().map_err(CliError::Schema)?;

    let live = relating_schema::build_live_document(
        &ctx.pool,
        ctx.graph.as_ref(),
        &ctx.project_id,
        &ctx.graph_path,
        Some(ctx.project.name.clone()),
    )
    .await?;
    let diff = relating_schema::diff_documents(&live, &document);
    match opts.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&diff).unwrap_or_default()
        ),
        OutputFormat::Text => {
            if diff.changes.is_empty() {
                println!("No changes: file matches the live schema.");
            } else {
                for change in &diff.changes {
                    println!(
                        "[{}] {} {}: {}",
                        if change.breaking { "BREAKING" } else { "safe" },
                        change.category,
                        change.subject,
                        change.detail
                    );
                }
            }
        }
    }
    Ok(if diff.has_breaking_changes() { 1 } else { 0 })
}

async fn cmd_eval_run(opts: &GlobalOpts, flags: &HashMap<String, String>) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let path = require_flag(flags, "questions")?;
    let ctx = open_context(opts, &project_id).await?;

    let strategy = match flags.get("strategy").map(|s| s.as_str()) {
        None | Some("hybrid") => RetrievalStrategy::Hybrid,
        Some("vector") => RetrievalStrategy::Vector,
        Some("keyword") => RetrievalStrategy::Keyword,
        Some(other) => return Err(CliError::Usage(format!("Unknown strategy '{}'", other))),
    };
    let min_grounded: f32 = match flags.get("min-grounded") {
        None => 0.5,
        Some(value) => value
            .parse()
            .map_err(|_| CliError::Usage("--min-grounded must be a number".to_string()))?,
    };

    let content = std::fs::read_to_string(&path)
        .map_err(|e| CliError::Usage(format!("Cannot read questions file: {}", e)))?;
    let questions: Vec<String> = content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if questions.is_empty() {
        return Err(CliError::Usage(
            "Questions file has no usable questions".to_string(),
        ));
    }

    let provider = relating_ai::create_ai_provider(&ctx.project.ai_config)?;
    let report =
        run_eval_questions(&ctx.pool, &provider, &ctx.project, &questions, strategy).await?;
    match opts.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "metrics": report.metrics,
                "prompt_version": report.prompt_version,
                "questions": report.questions,
            }))
            .unwrap_or_default()
        ),
        OutputFormat::Text => {
            println!(
                " grounded {}/{} ({:.2}), precision {:.2}, mean {:.0}ms [{}]",
                report.metrics.grounded_count,
                report.metrics.question_count,
                report.metrics.grounded_rate,
                report.metrics.mean_citation_precision,
                report.metrics.mean_latency_ms,
                report.prompt_version
            );
        }
    }
    Ok(if report.metrics.grounded_rate >= min_grounded {
        0
    } else {
        1
    })
}

async fn cmd_shadow_run(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let path = require_flag(flags, "queries")?;
    let ctx = open_context(opts, &project_id).await?;

    let conn = resolve_connection(&ctx, flags)
        .await?
        .ok_or_else(missing_connection_error)?;
    let password = relating_core::resolve_secret(&conn.auth_env_var).map_err(CliError::Usage)?;
    let uri = relating_migrate::BoltUri::parse(&conn.uri).map_err(CliError::Migrate)?;
    let mut config = relating_migrate::BoltConfig::new(uri, conn.username.clone(), password);
    config.database = conn.database.clone();
    let backend = relating_migrate::BoltBackend::new(config);

    let content = std::fs::read_to_string(&path)
        .map_err(|e| CliError::Usage(format!("Cannot read queries file: {}", e)))?;
    let queries: Vec<relating_shadow::ShadowQuery> = serde_json::from_str(&content)
        .map_err(|e| CliError::Usage(format!("Queries file is not valid JSON: {}", e)))?;

    let (comparisons, summary) = relating_shadow::replay_suite(
        ctx.graph.as_ref(),
        &ctx.graph_path,
        &backend,
        &queries,
        &relating_shadow::SuiteOptions::default(),
    )
    .await?;

    match opts.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "summary": summary,
                "comparisons": comparisons,
            }))
            .unwrap_or_default()
        ),
        OutputFormat::Text => {
            for comparison in &comparisons {
                println!(
                    "[{:?}] {}: {} (source {} rows, target {} rows, {:.1}x latency)",
                    comparison.verdict,
                    comparison.name,
                    comparison.detail,
                    comparison.source_rows,
                    comparison.target_rows,
                    comparison.latency_ratio
                );
            }
            println!(
                "Confidence: {:?} ({}/{})",
                summary.confidence, summary.matched, summary.total
            );
            for reason in &summary.confidence_reasons {
                println!("  - {}", reason);
            }
        }
    }
    Ok(if summary.confidence == relating_shadow::Confidence::Low {
        1
    } else {
        0
    })
}

async fn cmd_debug_trace(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let question = require_flag(flags, "question")?;
    let ctx = open_context(opts, &project_id).await?;

    let strategy = match flags.get("strategy").map(|s| s.as_str()) {
        None | Some("hybrid") => RetrievalStrategy::Hybrid,
        Some("vector") => RetrievalStrategy::Vector,
        Some("keyword") => RetrievalStrategy::Keyword,
        Some(other) => return Err(CliError::Usage(format!("Unknown strategy '{}'", other))),
    };

    let provider = relating_ai::create_ai_provider(&ctx.project.ai_config)?;
    let trace = relating_debug::build_trace(
        &ctx.pool,
        ctx.graph.as_ref(),
        &ctx.graph_path,
        &ctx.project,
        &provider,
        &question,
        strategy,
        5,
    )
    .await?;
    let mut rules = relating_debug::diagnose_rules(&trace);
    let ai_hypothesis = if flags.contains_key("no-ai") || trace.passages.is_empty() {
        None
    } else {
        match relating_debug::diagnose_ai(
            &provider,
            &trace,
            &rules,
            &ctx.project.ai_config.chat_model,
        )
        .await
        {
            Some(hypothesis) => Some(hypothesis),
            None => {
                rules.push(relating_debug::DiagnosisRule {
                    code: "DBG-I003".to_string(),
                    severity: relating_debug::Severity::Info,
                    message: "AI hypothesis unavailable: provider call failed".to_string(),
                });
                None
            }
        }
    };
    let likely_causes = relating_debug::likely_causes(&rules);

    match opts.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "trace": trace,
                "diagnosis": {"rules": rules, "likely_causes": likely_causes, "ai_hypothesis": ai_hypothesis},
            }))
            .unwrap_or_default()
        ),
        OutputFormat::Text => {
            println!("Question: {}", trace.question);
            println!(
                "Retrieval: {} passage(s) via {:?} on {} ({} {}d)",
                trace.passages.len(),
                trace.strategy,
                trace.profile_name,
                trace.embedding_model,
                trace.dimensions
            );
            println!("Grounded: {} | citations: {} | evidence: {} chars", trace.grounded, trace.citations.len(), trace.evidence_chars);
            println!("Answer: {}", trace.answer.chars().take(300).collect::<String>());
            println!("Likely causes:");
            for cause in &likely_causes {
                println!("  - {}", cause);
            }
            for rule in &rules {
                println!("[{:?}] {}: {}", rule.severity, rule.code, rule.message);
            }
            if let Some(hypothesis) = &ai_hypothesis {
                println!("AI hypothesis [{}]: {}", hypothesis.model, hypothesis.text);
            }
        }
    }
    Ok(0)
}

async fn cmd_synth_generate(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let out = require_flag(flags, "out")?;
    let ctx = open_context(opts, &project_id).await?;

    let scale: f64 = match flags.get("scale") {
        None => 1.0,
        Some(value) => value
            .parse()
            .map_err(|_| CliError::Usage("--scale must be a number 0.0-1.0".to_string()))?,
    };
    let seed: u64 = match flags.get("seed") {
        None => 0,
        Some(value) => value
            .parse()
            .map_err(|_| CliError::Usage("--seed must be a number".to_string()))?,
    };
    let mut edge_cases = Vec::new();
    if let Some(list) = flags.get("edge-cases") {
        for name in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let case = match name {
                "duplicates" => relating_synth::EdgeCase::Duplicates,
                "orphans" => relating_synth::EdgeCase::Orphans,
                "cycle" => relating_synth::EdgeCase::Cycle,
                "invalid_values" => relating_synth::EdgeCase::InvalidValues,
                other => return Err(CliError::Usage(format!("Unknown edge case '{}'", other))),
            };
            edge_cases.push(case);
        }
    }

    let profile = relating_synth::profile_graph(ctx.graph.as_ref(), &ctx.graph_path)?;
    let config = relating_synth::SynthConfig {
        scale,
        seed,
        edge_cases,
        max_nodes: 50_000,
    };
    let dataset = relating_synth::generate_dataset(&profile, &config)?;
    let json = serde_json::to_string_pretty(&dataset)
        .map_err(|e| CliError::Internal(format!("Failed to serialize dataset: {}", e)))?;
    std::fs::write(&out, json)
        .map_err(|e| CliError::Usage(format!("Cannot write '{}': {}", out, e)))?;

    if flags.contains_key("materialize") {
        let now = chrono::Utc::now();
        let target = relating_core::Project {
            id: ProjectId::new(),
            name: format!("{}-synth-{}", ctx.project.name, seed),
            description: format!("Synthetic fixture generated from '{}'", ctx.project.name),
            created_at: now,
            updated_at: now,
            ai_config: ctx.project.ai_config.clone(),
        };
        relating_metadata::insert_project(&ctx.pool, &target).await?;
        let target_path = opts
            .data_dir
            .join("projects")
            .join(target.id.to_string())
            .join("graph.rl");
        let outcome =
            relating_synth::materialize_dataset(ctx.graph.as_ref(), &target_path, &dataset)?;
        println!(
            "Generated {} nodes, {} rels -> {} (materialized {} nodes, {} rels into '{}')",
            dataset.stats.node_count,
            dataset.stats.rel_count,
            out,
            outcome.nodes_created,
            outcome.relationships_created,
            target.id
        );
    } else {
        println!(
            "Generated {} nodes, {} rels -> {}",
            dataset.stats.node_count, dataset.stats.rel_count, out
        );
    }
    for note in &dataset.notes {
        println!("note: {}", note);
    }
    Ok(0)
}

async fn cmd_advisor_analyze(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let project_id = parse_project_id(&require_flag(flags, "project")?)?;
    let path = require_flag(flags, "queries")?;
    let ctx = open_context(opts, &project_id).await?;

    let content = std::fs::read_to_string(&path)
        .map_err(|e| CliError::Usage(format!("Cannot read queries file: {}", e)))?;
    let queries: Vec<serde_json::Value> = serde_json::from_str(&content)
        .map_err(|e| CliError::Usage(format!("Queries file is not valid JSON: {}", e)))?;

    let mut query_reports = Vec::new();
    let mut warning_count = 0usize;
    for (i, query) in queries.iter().enumerate() {
        let name = query
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cypher = query
            .get("cypher")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CliError::Usage(format!("Query #{} has no 'cypher' string", i + 1)))?;
        let findings = relating_advisor::analyze_query(&name, cypher);
        warning_count += findings
            .iter()
            .filter(|f| f.severity == relating_advisor::Severity::Warning)
            .count();
        query_reports.push(serde_json::json!({"name": name, "findings": findings}));
    }

    let structure = relating_advisor::analyze_structure(ctx.graph.as_ref(), &ctx.graph_path)?;
    warning_count += structure
        .iter()
        .filter(|n| n.severity == relating_advisor::Severity::Warning)
        .count();
    let schema = ctx.graph.get_schema(&ctx.graph_path).unwrap_or_default();
    let sizing = relating_advisor::estimate_sizing(schema.node_count, schema.relationship_count);

    let measurements = if flags.contains_key("measure") {
        let conn = resolve_connection(&ctx, flags)
            .await?
            .ok_or_else(missing_connection_error)?;
        let password =
            relating_core::resolve_secret(&conn.auth_env_var).map_err(CliError::Usage)?;
        let uri = relating_migrate::BoltUri::parse(&conn.uri).map_err(CliError::Migrate)?;
        let mut config = relating_migrate::BoltConfig::new(uri, conn.username.clone(), password);
        config.database = conn.database.clone();
        let backend = relating_migrate::BoltBackend::new(config);
        let pairs: Vec<(String, String)> = queries
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let name = q
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let cypher = q
                    .get("cypher")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    (format!("query-{}", i + 1), cypher)
                } else {
                    (name, cypher)
                }
            })
            .collect();
        Some(
            relating_advisor::measure_target(&backend, &pairs)
                .await
                .map_err(CliError::Migrate)?,
        )
    } else {
        None
    };

    match opts.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "queries": query_reports,
                "structure": structure,
                "sizing": sizing,
                "measurements": measurements,
            }))
            .unwrap_or_default()
        ),
        OutputFormat::Text => {
            for report in &query_reports {
                println!("--- {} ---", report["name"].as_str().unwrap_or("?"));
                for finding in report["findings"].as_array().cloned().unwrap_or_default() {
                    println!(
                        "[{:?}] {}: {}",
                        finding["severity"].as_str().unwrap_or("?"),
                        finding["rule_id"].as_str().unwrap_or("?"),
                        finding["message"].as_str().unwrap_or("")
                    );
                }
            }
            println!("--- structure ---");
            for note in &structure {
                println!("[{:?}] {}: {}", note.severity, note.rule_id, note.message);
            }
            println!("--- sizing (estimate) ---");
            println!("{}", sizing.tier_hint);
        }
    }

    if flags.contains_key("fail-on-warning") && warning_count > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

async fn cmd_demo_seed(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let name = flags
        .get("name")
        .cloned()
        .unwrap_or_else(|| "demo".to_string());
    let pool = init_db(&opts.database_url)
        .await
        .map_err(|e| CliError::Project(format!("Cannot open metadata database: {}", e)))?;

    let now = chrono::Utc::now();
    let project = relating_core::Project {
        id: ProjectId::new(),
        name: name.clone(),
        description: "Seeded demo project (mock AI provider, offline-safe)".to_string(),
        created_at: now,
        updated_at: now,
        ai_config: relating_core::AiProviderConfig::mock(),
    };
    relating_metadata::insert_project(&pool, &project).await?;

    let graph_path = opts
        .data_dir
        .join("projects")
        .join(project.id.to_string())
        .join("graph.rl");
    let graph = relating_graph::RelateBackend::new();
    graph.ensure_store(&graph_path)?;
    graph.execute_write(
        &graph_path,
        "CREATE (a:City {uid: 'City:1', name: 'Lisbon'})-[:CONNECTED]->(b:City {uid: 'City:2', name: 'Porto'})",
        std::collections::HashMap::new(),
    )?;

    match opts.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "project_id": project.id.to_string(),
                "name": project.name,
            }))
            .unwrap_or_default()
        ),
        OutputFormat::Text => {
            println!("Seeded demo project '{}'", project.name);
            println!("project_id: {}", project.id);
        }
    }
    Ok(0)
}

async fn cmd_secrets_set(
    _opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let name = require_flag(flags, "name")?;
    let path = require_flag(flags, "from-file")?;
    let value = std::fs::read_to_string(&path)
        .map_err(|e| CliError::Usage(format!("Cannot read '{}': {}", path, e)))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(CliError::Usage(format!("'{}' is empty", path)));
    }
    relating_core::store_secret(&name, value).map_err(CliError::Check)?;
    println!(
        "Stored secret '{}' in the OS keychain (service 'relating')",
        name
    );
    Ok(0)
}

async fn cmd_secrets_check(
    _opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let name = require_flag(flags, "name")?;
    match relating_core::resolve_secret(&name) {
        Ok(_) => {
            println!("{}: SET", name);
            Ok(0)
        }
        Err(message) => {
            println!("{}: NOT SET", name);
            println!("{}", message);
            Ok(1)
        }
    }
}

async fn cmd_secrets_delete(
    _opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let name = require_flag(flags, "name")?;
    relating_core::delete_secret(&name).map_err(CliError::Check)?;
    println!("Deleted secret '{}' from the OS keychain", name);
    Ok(0)
}

async fn cmd_db_check_postgres(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let url = require_flag(flags, "url")?;
    #[cfg(feature = "postgres")]
    {
        let report = relating_metadata::check_postgres(&url).await?;
        match opts.format {
            OutputFormat::Json => println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            ),
            OutputFormat::Text => {
                println!("{}", report.message);
                if let Some(version) = &report.server_version {
                    println!("server: {}", version);
                }
                println!(
                    "tables: {}/{} present",
                    report.found_tables.len(),
                    report.expected_tables.len()
                );
            }
        }
        Ok(if report.schema_ready { 0 } else { 1 })
    }
    #[cfg(not(feature = "postgres"))]
    {
        let _ = (opts, url);
        Err(CliError::Usage(
            "PostgreSQL support is not enabled in this build; rebuild with --features postgres"
                .to_string(),
        ))
    }
}

async fn cmd_db_init_postgres(
    opts: &GlobalOpts,
    flags: &HashMap<String, String>,
) -> Result<i32, CliError> {
    let url = require_flag(flags, "url")?;
    #[cfg(feature = "postgres")]
    {
        let report = relating_metadata::ensure_pg_schema(&url).await?;
        match opts.format {
            OutputFormat::Json => println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            ),
            OutputFormat::Text => {
                println!("{}", report.message);
                println!(
                    "tables: {}/{} present",
                    report.found_tables.len(),
                    report.expected_tables.len()
                );
            }
        }
        Ok(if report.schema_ready { 0 } else { 1 })
    }
    #[cfg(not(feature = "postgres"))]
    {
        let _ = (opts, url);
        Err(CliError::Usage(
            "PostgreSQL support is not enabled in this build; rebuild with --features postgres"
                .to_string(),
        ))
    }
}

#[tokio::main]
async fn main() {
    // `.env` fills in unset variables only; real env always wins.
    relating_core::load_dotenv();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    let outcome: Result<i32, CliError> = async {
        let (opts, rest) = parse_global(&mut args)?;
        let (positionals, flags) = parse_flags(&rest)?;
        match positionals
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["validate"] => cmd_validate(&opts, &flags).await,
            ["aura", "analyze"] => cmd_aura_analyze(&opts, &flags).await,
            ["aura", "migrate"] => cmd_aura_migrate(&opts, &flags).await,
            ["portability", "scan"] => cmd_portability_scan(&opts, &flags).await,
            ["test", "graph"] => cmd_test_graph(&opts, &flags).await,
            ["schema", "export"] => cmd_schema_export(&opts, &flags).await,
            ["schema", "diff"] => cmd_schema_diff(&opts, &flags).await,
            ["eval", "run"] => cmd_eval_run(&opts, &flags).await,
            ["shadow", "run"] => cmd_shadow_run(&opts, &flags).await,
            ["debug", "trace"] => cmd_debug_trace(&opts, &flags).await,
            ["synth", "generate"] => cmd_synth_generate(&opts, &flags).await,
            ["advisor", "analyze"] => cmd_advisor_analyze(&opts, &flags).await,
            ["demo", "seed"] => cmd_demo_seed(&opts, &flags).await,
            ["secrets", "set"] => cmd_secrets_set(&opts, &flags).await,
            ["secrets", "check"] => cmd_secrets_check(&opts, &flags).await,
            ["secrets", "delete"] => cmd_secrets_delete(&opts, &flags).await,
            ["db", "check-postgres"] => cmd_db_check_postgres(&opts, &flags).await,
            ["db", "init-postgres"] => cmd_db_init_postgres(&opts, &flags).await,
            _ => Err(CliError::Usage(format!(
                "Unknown command: {}",
                positionals.join(" ")
            ))),
        }
    }
    .await;

    match outcome {
        Ok(code) => std::process::exit(code),
        Err(CliError::Usage(message)) => {
            eprintln!("Error: {}", message);
            eprintln!("Run `relating --help` for usage.");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(2);
        }
    }
}
