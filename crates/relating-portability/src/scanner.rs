use crate::error::PortError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLanguage {
    Python,
    TypeScript,
    JavaScript,
    Java,
    Go,
    Rust,
    Cypher,
    Unknown,
}

impl SourceLanguage {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "py" => Some(Self::Python),
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "java" => Some(Self::Java),
            "go" => Some(Self::Go),
            "rs" => Some(Self::Rust),
            "cypher" | "cyp" => Some(Self::Cypher),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCandidate {
    pub file: String,
    pub line: u32,
    pub language: SourceLanguage,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSignal {
    pub file: String,
    pub line: u32,
    pub kind: SignalKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    DriverImport,
    ConnectionUri,
    Transaction,
    ApocUsage,
    ConstraintDdl,
    IndexDdl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoScan {
    pub root: String,
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub queries: Vec<QueryCandidate>,
    pub signals: Vec<CodeSignal>,
}

pub const MAX_FILE_BYTES: u64 = 1_000_000;
pub const MAX_FILES: usize = 5000;
pub const MAX_QUERIES: usize = 2000;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
];

fn string_literal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?s)("""(?P<py3>.*?)"\"""|'''(?P<py3s>.*?)'''|"(?P<dquote>(?:[^"\\]|\\.)*)"|'(?P<squote>(?:[^'\\]|\\.)*)'|`(?P<tick>(?:[^`\\]|\\.)*)`)"#,
        )
        .expect("valid string literal regex")
    })
}

fn cypher_keyword_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(MATCH|CREATE|MERGE|RETURN|UNWIND|WITH|OPTIONAL\s+MATCH|DELETE|DETACH|SET|REMOVE|CALL|LOAD\s+CSV|USING\s+PERIODIC\s+COMMIT)\b",
        )
        .expect("valid cypher regex")
    })
}

fn driver_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(from\s+neo4j\s+import|import\s+neo4j|from\s+relate|require\(['"]neo4j-driver['"]\)|from\s+['"]neo4j-driver['"]|@relate/relate|relate-db|use\s+neo4rs|use\s+relate::|"github\.com/neo4j/neo4j-go-driver|neo4j\.NewDriver|GraphDatabase\.driver)"#,
        )
        .expect("valid driver regex")
    })
}

fn uri_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\b((?:bolt|neo4j)(?:\+s|\+ssc)?://[^\s"'`]+)"#).expect("valid uri regex")
    })
}

fn tx_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(write_transaction|read_transaction|session\.begin_transaction|tx\.run|transaction\.run|\.beginTx\(|BEGIN\s+TRANSACTION)\b",
        )
        .expect("valid tx regex")
    })
}

fn apoc_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bapoc\.[a-zA-Z0-9_.]+").expect("valid apoc regex"))
}

fn ddl_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bCREATE\s+(CONSTRAINT|INDEX)\b").expect("valid ddl regex"))
}

fn line_number_of(text: &str, byte_offset: usize) -> u32 {
    text[..byte_offset.min(text.len())]
        .chars()
        .filter(|c| *c == '\n')
        .count() as u32
        + 1
}

fn looks_like_cypher(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 8 || trimmed.len() > 20000 {
        return false;
    }
    // Needs a Cypher keyword plus a graph-y character to avoid prose matches.
    cypher_keyword_re().is_match(trimmed) && trimmed.chars().any(|c| "(){}[]:-".contains(c))
}

fn collect_files(
    root: &Path,
    out: &mut Vec<PathBuf>,
    skipped: &mut usize,
) -> Result<(), PortError> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_FILES {
            return Ok(());
        }
        let entries = std::fs::read_dir(&dir).map_err(|e| {
            PortError::Scan(format!("Cannot read directory '{}': {}", dir.display(), e))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if name.starts_with('.') && name != "." && name != ".." {
                    *skipped += 1;
                    continue;
                }
                if SKIP_DIRS.contains(&name.as_str()) {
                    *skipped += 1;
                    continue;
                }
                stack.push(path);
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if SourceLanguage::from_extension(ext).is_none() {
                    continue;
                }
                let size = entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
                if size > MAX_FILE_BYTES {
                    *skipped += 1;
                    continue;
                }
                out.push(path);
                if out.len() >= MAX_FILES {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn scan_file(root: &Path, path: &Path, scan: &mut RepoScan) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => {
            scan.files_skipped += 1; // binary or unreadable
            return;
        }
    };
    let rel = path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let language = SourceLanguage::from_extension(ext).unwrap_or(SourceLanguage::Unknown);
    scan.files_scanned += 1;

    // Whole-file Cypher sources.
    if language == SourceLanguage::Cypher {
        for (idx, stmt) in crate::cypher::split_statements(&content)
            .into_iter()
            .enumerate()
        {
            if scan.queries.len() >= MAX_QUERIES {
                return;
            }
            if stmt.trim().is_empty() {
                continue;
            }
            scan.queries.push(QueryCandidate {
                file: rel.clone(),
                line: 1 + idx as u32,
                language,
                query: stmt,
            });
        }
        return;
    }

    // String literals that look like Cypher.
    for caps in string_literal_re().captures_iter(&content) {
        if scan.queries.len() >= MAX_QUERIES {
            break;
        }
        let whole = caps.get(0).unwrap();
        let inner = caps
            .name("py3")
            .or_else(|| caps.name("py3s"))
            .or_else(|| caps.name("dquote"))
            .or_else(|| caps.name("squote"))
            .or_else(|| caps.name("tick"))
            .map(|m| m.as_str())
            .unwrap_or("");
        // Skip f-string/format placeholders noise but keep the literal.
        let normalized = inner
            .replace("\\n", "\n")
            .replace("\\\"", "\"")
            .replace("\\'", "'");
        if looks_like_cypher(&normalized) {
            scan.queries.push(QueryCandidate {
                file: rel.clone(),
                line: line_number_of(&content, whole.start()),
                language,
                query: normalized.trim().to_string(),
            });
        }
    }

    // Line-based code signals.
    let mut seen_signals: HashSet<(u32, u8)> = HashSet::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx as u32 + 1;
        let mut push = |kind: SignalKind, disc: u8| {
            if seen_signals.insert((line_no, disc)) {
                scan.signals.push(CodeSignal {
                    file: rel.clone(),
                    line: line_no,
                    kind,
                    text: line.trim().chars().take(200).collect(),
                });
            }
        };
        if driver_re().is_match(line) {
            push(SignalKind::DriverImport, 0);
        }
        if uri_re().is_match(line) {
            push(SignalKind::ConnectionUri, 1);
        }
        if tx_re().is_match(line) {
            push(SignalKind::Transaction, 2);
        }
        if apoc_re().is_match(line) {
            push(SignalKind::ApocUsage, 3);
        }
        if ddl_re().is_match(line) {
            if line.to_uppercase().contains("CONSTRAINT") {
                push(SignalKind::ConstraintDdl, 4);
            } else {
                push(SignalKind::IndexDdl, 5);
            }
        }
    }
}

pub fn scan_repository(root: &Path) -> Result<RepoScan, PortError> {
    if !root.is_dir() {
        return Err(PortError::Scan(format!(
            "'{}' is not a readable directory",
            root.display()
        )));
    }
    let root = root
        .canonicalize()
        .map_err(|e| PortError::Scan(format!("Cannot resolve '{}': {}", root.display(), e)))?;

    let mut files = Vec::new();
    let mut skipped = 0usize;
    collect_files(&root, &mut files, &mut skipped)?;
    files.sort();

    let mut scan = RepoScan {
        root: root.to_string_lossy().to_string(),
        files_scanned: 0,
        files_skipped: skipped,
        queries: Vec::new(),
        signals: Vec::new(),
    };
    for path in files {
        // Stay inside the scanned root (no symlink escapes).
        if let Ok(canonical) = path.canonicalize() {
            if !canonical.starts_with(&root) {
                scan.files_skipped += 1;
                continue;
            }
        }
        scan_file(&root, &path, &mut scan);
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let app_py = dir.path().join("app.py");
        let mut f = std::fs::File::create(&app_py).unwrap();
        writeln!(
            f,
            "from neo4j import GraphDatabase\n\
             URI = \"bolt://localhost:7687\"\n\
             def get_friends(tx, name):\n    \
             return tx.run(\"MATCH (p:Person {{name: $name}})-[:KNOWS]->(f) RETURN f.name AS friend\", name=name)\n"
        )
        .unwrap();
        let queries_ts = dir.path().join("queries.ts");
        let mut f = std::fs::File::create(&queries_ts).unwrap();
        writeln!(
            f,
            "import neo4j from 'neo4j-driver';\n\
             const q = `MATCH (n) RETURN n LIMIT 10`;\n\
             const apoc = \"CALL apoc.meta.schema() YIELD value RETURN value\";\n"
        )
        .unwrap();
        let seed = dir.path().join("seed.cypher");
        std::fs::write(
            &seed,
            "CREATE (n:Person {name: 'Ada'});\nMATCH (n) RETURN n;\n",
        )
        .unwrap();
        let ignored = dir.path().join("notes.txt");
        std::fs::write(&ignored, "MATCH nothing in a text file").unwrap();
        dir
    }

    #[test]
    fn test_scan_finds_queries_and_signals() {
        let dir = fixture_repo();
        let scan = scan_repository(dir.path()).unwrap();
        assert_eq!(scan.files_scanned, 3);
        // 1 python + 2 typescript string queries + 2 cypher statements.
        assert_eq!(scan.queries.len(), 5);
        assert!(scan
            .queries
            .iter()
            .any(|q| q.file == "app.py" && q.query.contains("KNOWS")));
        assert!(scan
            .signals
            .iter()
            .any(|s| s.kind == SignalKind::DriverImport && s.file == "app.py"));
        assert!(scan
            .signals
            .iter()
            .any(|s| s.kind == SignalKind::ConnectionUri));
        assert!(scan
            .signals
            .iter()
            .any(|s| s.kind == SignalKind::Transaction));
        assert!(scan.signals.iter().any(|s| s.kind == SignalKind::ApocUsage));
    }

    #[test]
    fn test_scan_rejects_missing_dir() {
        assert!(scan_repository(Path::new("/nonexistent-xyz-123")).is_err());
    }

    #[test]
    fn test_prose_is_not_cypher() {
        assert!(!looks_like_cypher("hello world"));
        assert!(!looks_like_cypher("MATCH"));
        assert!(looks_like_cypher("MATCH (n) RETURN n"));
    }
}
