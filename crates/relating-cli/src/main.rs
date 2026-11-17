use relating_cli::checks::{
    check_aura_compat, check_conformance_gate, check_index_health, check_integrity, CheckOutcome,
    Context,
};
use relating_cli::error::CliError;
use relating_cli::eval_runner::run_eval_questions;
use relating_cli::output::{render_validate, OutputFormat, ValidateReport};
use relating_core::{AuraConnectionId, ProjectId, RetrievalStrategy};
use relating_graph::GraphBackend;
use relating_metadata::{get_aura_connection, init_db};
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
    eprintln!("  aura analyze --project ID --connection CID");
    eprintln!("  test graph --project ID [--revision N]");
    eprintln!("  schema export --project ID --out DIR");
    eprintln!("  schema diff --project ID --file schema.json");
    eprintln!("  eval run --project ID --questions FILE [--strategy S] [--min-grounded F]");
    eprintln!("  shadow run --project ID --connection CID --queries FILE");
    eprintln!("  debug trace --project ID --question TEXT [--strategy S] [--no-ai]");
    eprintln!("  synth generate --project ID --out FILE [--scale F] [--seed N] [--edge-cases a,b] [--materialize]");
    eprintln!("  advisor analyze --project ID --queries FILE [--connection CID] [--measure] [--fail-on-warning]");
    eprintln!("  demo seed [--name NAME]   (create a seeded demo project; prints its ID)");
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
        // Aura compatibility when a connection is supplied, else an honest skip.
        match flags.get("connection") {
            Some(conn_id_str) => {
                let conn_id = AuraConnectionId::parse(conn_id_str)
                    .map_err(|e| CliError::Usage(format!("Invalid connection ID: {}", e)))?;
                let conn = get_aura_connection(&ctx.pool, &conn_id)
                    .await?
                    .ok_or_else(|| CliError::Usage("Aura connection not found".to_string()))?;
                gates.push(check_aura_compat(&ctx, Some(&conn)).await?);
            }
            None => {
                let mut skipped = CheckOutcome::pass("aura-compat");
                skipped
                    .warnings
                    .push("Skipped: pass --connection to include Aura analysis".to_string());
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
    let conn_id_str = require_flag(flags, "connection")?;
    let ctx = open_context(opts, &project_id).await?;
    let conn_id = AuraConnectionId::parse(&conn_id_str)
        .map_err(|e| CliError::Usage(format!("Invalid connection ID: {}", e)))?;
    let conn = get_aura_connection(&ctx.pool, &conn_id)
        .await?
        .ok_or_else(|| CliError::Usage("Aura connection not found".to_string()))?;

    let gate = check_aura_compat(&ctx, Some(&conn)).await?;
    let report = ValidateReport {
        passed: gate.passed,
        gates: vec![gate],
    };
    println!("{}", render_validate(&report, opts.format));
    Ok(if report.passed { 0 } else { 1 })
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
    let conn_id_str = require_flag(flags, "connection")?;
    let path = require_flag(flags, "queries")?;
    let ctx = open_context(opts, &project_id).await?;

    let conn_id = AuraConnectionId::parse(&conn_id_str)
        .map_err(|e| CliError::Usage(format!("Invalid connection ID: {}", e)))?;
    let conn = get_aura_connection(&ctx.pool, &conn_id)
        .await?
        .ok_or_else(|| CliError::Usage("Aura connection not found".to_string()))?;
    let password = std::env::var(&conn.auth_env_var).map_err(|_| {
        CliError::Usage(format!(
            "Password environment variable '{}' is not set",
            conn.auth_env_var
        ))
    })?;
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
        let conn_id_str = require_flag(flags, "connection")?;
        let conn_id = AuraConnectionId::parse(&conn_id_str)
            .map_err(|e| CliError::Usage(format!("Invalid connection ID: {}", e)))?;
        let conn = get_aura_connection(&ctx.pool, &conn_id)
            .await?
            .ok_or_else(|| CliError::Usage("Aura connection not found".to_string()))?;
        let password = std::env::var(&conn.auth_env_var).map_err(|_| {
            CliError::Usage(format!(
                "Password environment variable '{}' is not set",
                conn.auth_env_var
            ))
        })?;
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

#[tokio::main]
async fn main() {
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
            ["test", "graph"] => cmd_test_graph(&opts, &flags).await,
            ["schema", "export"] => cmd_schema_export(&opts, &flags).await,
            ["schema", "diff"] => cmd_schema_diff(&opts, &flags).await,
            ["eval", "run"] => cmd_eval_run(&opts, &flags).await,
            ["shadow", "run"] => cmd_shadow_run(&opts, &flags).await,
            ["debug", "trace"] => cmd_debug_trace(&opts, &flags).await,
            ["synth", "generate"] => cmd_synth_generate(&opts, &flags).await,
            ["advisor", "analyze"] => cmd_advisor_analyze(&opts, &flags).await,
            ["demo", "seed"] => cmd_demo_seed(&opts, &flags).await,
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
