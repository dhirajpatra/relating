use crate::error::MigrateError;
use crate::packstream::{decode_message, encode_message, BoltValue};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::warn;

const HANDSHAKE_MAGIC: [u8; 4] = [0x60, 0x60, 0xB0, 0x17];
const MAX_CHUNK_LEN: usize = 16384;
const DEFAULT_PORT: u16 = 7687;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoltUri {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
}

impl BoltUri {
    pub fn parse(raw: &str) -> Result<Self, MigrateError> {
        let raw = raw.trim();
        let (scheme, rest) = raw.split_once("://").ok_or_else(|| {
            MigrateError::Connection(format!(
                "Invalid Neo4j URI '{}': expected scheme://host[:port]",
                raw
            ))
        })?;

        let use_tls = match scheme {
            "bolt" | "neo4j" => false,
            "bolt+s" | "neo4j+s" => true,
            "bolt+ssc" | "neo4j+ssc" => {
                return Err(MigrateError::Connection(
                    "Self-signed certificates (+ssc) are not accepted; use +s with a trusted certificate".to_string(),
                ))
            }
            _ => {
                return Err(MigrateError::Connection(format!(
                    "Unsupported Neo4j URI scheme '{}': use bolt://, bolt+s://, neo4j:// or neo4j+s://",
                    scheme
                )))
            }
        };

        if rest.contains('@') {
            return Err(MigrateError::Connection(
                "Credentials must not be embedded in the URI; supply the username and password separately".to_string(),
            ));
        }

        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if !p.contains('/') && !p.contains('[') => {
                let port = p.parse::<u16>().map_err(|_| {
                    MigrateError::Connection(format!("Invalid port '{}' in URI", p))
                })?;
                (h, port)
            }
            _ => (rest, DEFAULT_PORT),
        };

        let host = host
            .trim()
            .trim_matches(|c| c == '[' || c == ']')
            .trim_end_matches('/');
        if host.is_empty() {
            return Err(MigrateError::Connection("URI has no host".to_string()));
        }

        Ok(Self {
            scheme: scheme.to_string(),
            host: host.to_string(),
            port,
            use_tls,
        })
    }

    pub fn is_routing_scheme(&self) -> bool {
        self.scheme.starts_with("neo4j")
    }
}

#[derive(Debug, Clone)]
pub struct BoltConfig {
    pub uri: BoltUri,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    pub connect_timeout: Duration,
    pub query_timeout: Duration,
}

impl BoltConfig {
    pub fn new(uri: BoltUri, username: String, password: String) -> Self {
        Self {
            uri,
            username,
            password,
            database: None,
            connect_timeout: Duration::from_secs(10),
            query_timeout: Duration::from_secs(60),
        }
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, MigrateError> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    if let Some(config) = CONFIG.get() {
        return Ok(config.clone());
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let config = Arc::new(config);
    let _ = CONFIG.set(config.clone());
    Ok(config)
}

enum BoltStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl BoltStream {
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), MigrateError> {
        let res = match self {
            BoltStream::Plain(s) => s.read_exact(buf).await,
            BoltStream::Tls(s) => s.read_exact(buf).await,
        };
        res.map(|_| ())
            .map_err(|e| MigrateError::Connection(format!("Read failed: {}", e)))
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), MigrateError> {
        let res = match self {
            BoltStream::Plain(s) => s.write_all(buf).await,
            BoltStream::Tls(s) => s.write_all(buf).await,
        };
        res.map_err(|e| MigrateError::Connection(format!("Write failed: {}", e)))
    }

    async fn flush(&mut self) -> Result<(), MigrateError> {
        let res = match self {
            BoltStream::Plain(s) => s.flush().await,
            BoltStream::Tls(s) => s.flush().await,
        };
        res.map_err(|e| MigrateError::Connection(format!("Flush failed: {}", e)))
    }
}

pub fn chunk_message(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 8);
    for piece in raw.chunks(MAX_CHUNK_LEN.max(1)) {
        out.extend_from_slice(&(piece.len() as u16).to_be_bytes());
        out.extend_from_slice(piece);
    }
    out.extend_from_slice(&[0x00, 0x00]);
    out
}

pub struct BoltClient {
    stream: BoltStream,
    pub negotiated_major: u8,
    pub negotiated_minor: u8,
    pub server_agent: String,
    query_timeout: Duration,
}

fn version_to_u32(major: u8, minor: u8) -> u32 {
    ((minor as u32) << 8) | major as u32
}

/// Parsed Bolt routing table (v4 top-level shape and v5 `rt`-nested shape).
#[derive(Debug, Clone, Default)]
pub struct RoutingTable {
    pub ttl_secs: u64,
    pub database: Option<String>,
    pub writers: Vec<String>,
    pub readers: Vec<String>,
    pub routers: Vec<String>,
}

fn parse_routing_table(meta: &BTreeMap<String, BoltValue>) -> Result<RoutingTable, MigrateError> {
    let malformed = || MigrateError::Protocol("Malformed ROUTE response".to_string());
    // Bolt 4.x serves the table at top level; Bolt 5 nests it under `rt`.
    let meta = match meta.get("servers") {
        Some(_) => meta,
        None => match meta.get("rt") {
            Some(BoltValue::Map(rt)) => rt,
            _ => return Err(malformed()),
        },
    };
    let ttl_secs = meta
        .get("ttl")
        .and_then(|v| v.as_int())
        .unwrap_or(300)
        .max(0) as u64;
    let database = meta
        .get("db")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let servers = match meta.get("servers") {
        Some(BoltValue::List(servers)) => servers,
        _ => return Err(malformed()),
    };
    let mut table = RoutingTable {
        ttl_secs,
        database,
        ..Default::default()
    };
    for server in servers {
        let entry = match server {
            BoltValue::Map(entry) => entry,
            _ => continue,
        };
        let role = entry
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let addresses: Vec<String> = match entry.get("addresses") {
            Some(BoltValue::List(addresses)) => addresses
                .iter()
                .filter_map(|a| a.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        // v4 uses WRITE/READ/ROUTE; accept the v5 WRITER/READER/ROUTER
        // spellings too so one parser serves every server we meet.
        match role.as_str() {
            "WRITE" | "WRITER" => table.writers.extend(addresses),
            "READ" | "READER" => table.readers.extend(addresses),
            "ROUTE" | "ROUTER" => table.routers.extend(addresses),
            _ => {}
        }
    }
    Ok(table)
}

/// Major version from a Bolt HELLO `server` agent string such as
/// `Neo4j/5.27-aura`. Unknown shapes yield 0 (oldest behavior first).
fn server_major(agent: &str) -> u64 {
    agent
        .split('/')
        .nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|m| m.parse().ok())
        .unwrap_or(0)
}

fn parse_host_port(address: &str) -> (String, u16) {
    match address.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(port) => (host.to_string(), port),
            Err(_) => (address.to_string(), DEFAULT_PORT),
        },
        None => (address.to_string(), DEFAULT_PORT),
    }
}

impl BoltClient {
    pub async fn connect(config: &BoltConfig) -> Result<Self, MigrateError> {
        let mut client = Self::connect_direct(config).await?;
        if !config.uri.is_routing_scheme() {
            return Ok(client);
        }
        // neo4j:// URIs follow cluster routing like official drivers. The
        // front door of multi-tenant/clustered Aura does not host user
        // databases itself, so a bare direct connection fails later with
        // DatabaseNotFound; ROUTE reveals the member to talk to instead.
        let table = match client
            .route(&config.uri.host, config.database.as_deref())
            .await
        {
            Ok(table) => table,
            Err(e) => {
                warn!(
                    "ROUTE discovery failed ({}); continuing with a direct connection",
                    e
                );
                return Ok(client);
            }
        };
        let Some(target) = table
            .writers
            .first()
            .or_else(|| table.readers.first())
            .cloned()
        else {
            warn!("ROUTE table has no writers or readers; continuing with a direct connection");
            return Ok(client);
        };
        let (host, port) = parse_host_port(&target);
        let mut routed = config.clone();
        // One hop only: writer addresses use the direct scheme, which never
        // routes again. TLS follows the original URI.
        routed.uri = BoltUri {
            scheme: if config.uri.use_tls {
                "bolt+s".to_string()
            } else {
                "bolt".to_string()
            },
            host,
            port,
            use_tls: config.uri.use_tls,
        };
        Self::connect_direct(&routed).await
    }

    async fn connect_direct(config: &BoltConfig) -> Result<Self, MigrateError> {
        let addr = format!("{}:{}", config.uri.host, config.uri.port);
        let tcp = tokio::time::timeout(config.connect_timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| MigrateError::Timeout(format!("Timed out connecting to {}", addr)))?
            .map_err(|e| MigrateError::Connection(format!("Cannot reach {}: {}", addr, e)))?;

        let mut stream = if config.uri.use_tls {
            let tls = tls_config()?;
            let connector = tokio_rustls::TlsConnector::from(tls);
            let server_name = rustls::pki_types::ServerName::try_from(config.uri.host.clone())
                .map_err(|_| {
                    MigrateError::Tls(format!("Invalid TLS server name '{}'", config.uri.host))
                })?;
            let tls_stream =
                tokio::time::timeout(config.connect_timeout, connector.connect(server_name, tcp))
                    .await
                    .map_err(|_| MigrateError::Timeout("TLS handshake timed out".to_string()))?
                    .map_err(|e| MigrateError::Tls(format!("TLS handshake failed: {}", e)))?;
            BoltStream::Tls(Box::new(tls_stream))
        } else {
            BoltStream::Plain(tcp)
        };

        // Bolt handshake: magic + 4 version proposals, highest first.
        let mut hello = Vec::with_capacity(20);
        hello.extend_from_slice(&HANDSHAKE_MAGIC);
        for (major, minor) in [(4u8, 4u8), (4, 3), (4, 2), (3, 0)] {
            hello.extend_from_slice(&version_to_u32(major, minor).to_be_bytes());
        }
        stream.write_all(&hello).await?;
        stream.flush().await?;

        let mut version_buf = [0u8; 4];
        stream.read_exact(&mut version_buf).await?;
        let negotiated = u32::from_be_bytes(version_buf);
        if negotiated == 0 {
            return Err(MigrateError::Protocol(
                "Server supports none of the offered Bolt versions (4.4, 4.3, 4.2, 3.0)"
                    .to_string(),
            ));
        }
        let negotiated_major = (negotiated & 0xFF) as u8;
        let negotiated_minor = ((negotiated >> 8) & 0xFF) as u8;

        let mut client = Self {
            stream,
            negotiated_major,
            negotiated_minor,
            server_agent: String::new(),
            query_timeout: config.query_timeout,
        };

        // HELLO with basic auth.
        let mut auth = BTreeMap::new();
        auth.insert(
            "user_agent".to_string(),
            BoltValue::String("relating-migrate/0.1.0".to_string()),
        );
        auth.insert("scheme".to_string(), BoltValue::String("basic".to_string()));
        auth.insert(
            "principal".to_string(),
            BoltValue::String(config.username.clone()),
        );
        auth.insert(
            "credentials".to_string(),
            BoltValue::String(config.password.clone()),
        );
        let meta = client.exchange(0x01, &[BoltValue::Map(auth)]).await?;
        if let Some(BoltValue::String(server)) = meta.get("server") {
            client.server_agent = server.clone();
        }
        Ok(client)
    }

    async fn send_message(&mut self, tag: u8, fields: &[BoltValue]) -> Result<(), MigrateError> {
        let raw = encode_message(tag, fields)?;
        let framed = chunk_message(&raw);
        self.stream.write_all(&framed).await?;
        self.stream.flush().await
    }

    async fn recv_message(&mut self) -> Result<(u8, Vec<BoltValue>), MigrateError> {
        let mut payload = Vec::new();
        loop {
            let mut len_buf = [0u8; 2];
            self.stream.read_exact(&mut len_buf).await?;
            let len = u16::from_be_bytes(len_buf) as usize;
            if len == 0 {
                break;
            }
            let mut chunk = vec![0u8; len];
            self.stream.read_exact(&mut chunk).await?;
            payload.extend_from_slice(&chunk);
        }
        decode_message(&payload)
    }

    async fn reset_after_failure(&mut self) {
        let _ = self.send_message(0x0F, &[]).await;
        let _ = self.recv_message().await;
    }

    /// RUN + PULL for one statement; returns record rows and the SUCCESS metadata.
    pub async fn run(
        &mut self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<(Vec<Vec<BoltValue>>, BTreeMap<String, BoltValue>), MigrateError> {
        tokio::time::timeout(self.query_timeout, self.run_inner(query, params))
            .await
            .map_err(|_| MigrateError::Timeout("Query timed out".to_string()))?
    }

    async fn run_inner(
        &mut self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<(Vec<Vec<BoltValue>>, BTreeMap<String, BoltValue>), MigrateError> {
        // Delegate to the fields-aware loop: real servers answer the pipelined
        // RUN with its own SUCCESS (carrying `fields`) before any RECORD, so
        // returning on the first SUCCESS would drop every row — and the write
        // stats with them. The shared loop skips the RUN acknowledgement and
        // returns the final PULL SUCCESS instead.
        let (_columns, rows, meta) = self.run_records_with_fields_inner(query, params).await?;
        Ok((rows, meta))
    }

    /// RUN + PULL that also returns the result column names.
    ///
    /// This pipelines RUN and PULL, then reads every response: the RUN SUCCESS
    /// (which carries `fields` on real servers) is captured and reading
    /// continues through RECORDs to the final SUCCESS. [`BoltClient::run`]
    /// delegates here and drops the columns, so every read path shares the
    /// real-server framing.
    pub async fn run_records_with_fields(
        &mut self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<
        (
            Vec<String>,
            Vec<Vec<BoltValue>>,
            BTreeMap<String, BoltValue>,
        ),
        MigrateError,
    > {
        tokio::time::timeout(
            self.query_timeout,
            self.run_records_with_fields_inner(query, params),
        )
        .await
        .map_err(|_| MigrateError::Timeout("Query timed out".to_string()))?
    }

    async fn run_records_with_fields_inner(
        &mut self,
        query: &str,
        params: BTreeMap<String, BoltValue>,
    ) -> Result<
        (
            Vec<String>,
            Vec<Vec<BoltValue>>,
            BTreeMap<String, BoltValue>,
        ),
        MigrateError,
    > {
        let empty = BTreeMap::new();
        self.send_message(
            0x10,
            &[
                BoltValue::String(query.to_string()),
                BoltValue::Map(params),
                BoltValue::Map(empty),
            ],
        )
        .await?;

        let mut pull_extra = BTreeMap::new();
        pull_extra.insert("n".to_string(), BoltValue::Int(-1));
        self.send_message(0x3F, &[BoltValue::Map(pull_extra)])
            .await?;

        fn columns_of(meta: &BTreeMap<String, BoltValue>) -> Vec<String> {
            match meta.get("fields") {
                Some(BoltValue::List(names)) => names
                    .iter()
                    .filter_map(|name| name.as_str().map(|s| s.to_string()))
                    .collect(),
                _ => Vec::new(),
            }
        }

        let mut columns = Vec::new();
        let mut rows = Vec::new();
        loop {
            let (tag, fields) = self.recv_message().await?;
            match tag {
                0x71 => match fields.as_slice() {
                    [BoltValue::List(cols)] => rows.push(cols.clone()),
                    _ => {
                        return Err(MigrateError::Protocol(
                            "Malformed RECORD message".to_string(),
                        ));
                    }
                },
                0x70 => match fields.as_slice() {
                    [BoltValue::Map(meta)] => {
                        // Real servers send RUN SUCCESS (`fields`, never
                        // `stats`) before any RECORD, then the final PULL
                        // SUCCESS (no `fields`). Test fakes may attach
                        // `fields` to the final SUCCESS instead; accept them
                        // from whichever SUCCESS names them.
                        let found = columns_of(meta);
                        let has_fields = !found.is_empty();
                        if has_fields {
                            columns = found;
                        }
                        let is_run_response =
                            has_fields && !meta.contains_key("stats") && rows.is_empty();
                        if is_run_response {
                            continue;
                        }
                        return Ok((columns, rows, meta.clone()));
                    }
                    _ => {
                        return Err(MigrateError::Protocol(
                            "Malformed SUCCESS message".to_string(),
                        ));
                    }
                },
                0x7F => return Err(self.protocol_failure(&fields).await),
                0x7E => {
                    return Err(MigrateError::Protocol(
                        "Server ignored the request (previous failure)".to_string(),
                    ));
                }
                _ => {
                    return Err(MigrateError::Protocol(format!(
                        "Unexpected response message {:#04x}",
                        tag
                    )));
                }
            }
        }
    }

    async fn protocol_failure(&mut self, fields: &[BoltValue]) -> MigrateError {
        let (code, message) = failure_details(fields);
        self.reset_after_failure().await;
        if code.contains("Unauthorized") || code.contains("Authentication") {
            MigrateError::Auth(message)
        } else {
            MigrateError::Protocol(format!("{}: {}", code, message))
        }
    }

    /// Single request/response exchange (HELLO, BEGIN, COMMIT, ...).
    async fn exchange(
        &mut self,
        tag: u8,
        fields: &[BoltValue],
    ) -> Result<BTreeMap<String, BoltValue>, MigrateError> {
        self.send_message(tag, fields).await?;
        let (resp_tag, resp_fields) = self.recv_message().await?;
        match resp_tag {
            0x70 => match resp_fields.as_slice() {
                [BoltValue::Map(meta)] => Ok(meta.clone()),
                _ => Err(MigrateError::Protocol(
                    "Malformed SUCCESS message".to_string(),
                )),
            },
            0x7F => {
                let (code, message) = failure_details(&resp_fields);
                self.reset_after_failure().await;
                if code.contains("Unauthorized") || code.contains("Authentication") {
                    return Err(MigrateError::Auth(message));
                }
                Err(MigrateError::Protocol(format!("{}: {}", code, message)))
            }
            _ => Err(MigrateError::Protocol(format!(
                "Unexpected response message {:#04x}",
                resp_tag
            ))),
        }
    }

    /// ROUTE request (three-field form): routing context with the
    /// originally requested address, bookmarks, and database.
    ///
    /// Neo4j 4.x wants the database as a bare string (`ROUTE(ctx, [], "db")`)
    /// while Neo4j 5.x servers reject that with
    /// `Expected MAP but got STRING` and want a database-context map
    /// (`ROUTE(ctx, [], {db: "db"})`) — even on a Bolt 4.4 connection. A
    /// failed ROUTE used to mean "fall back to a direct connection", which
    /// lands on arbitrary front-door nodes that may not host the database,
    /// surfacing as intermittent `DatabaseNotFound`. So the client tries the
    /// form matching the server's major version first, then the other form;
    /// only when both fail does the caller fall back to direct. Each attempt
    /// is self-cleaning (`exchange` RESETs after FAILURE).
    pub async fn route(
        &mut self,
        address: &str,
        database: Option<&str>,
    ) -> Result<RoutingTable, MigrateError> {
        let mut context = BTreeMap::new();
        context.insert(
            "address".to_string(),
            BoltValue::String(address.to_string()),
        );
        let context = BoltValue::Map(context);
        let bookmarks = BoltValue::List(Vec::new());

        // v5 form first on Neo4j 5+ (read from the HELLO agent string),
        // v4 form first everywhere else; the other form is the fallback.
        let v5_first = server_major(&self.server_agent) >= 5;
        let order = if v5_first { [true, false] } else { [false, true] };
        let mut last_err: Option<MigrateError> = None;
        for use_v5 in order {
            let db_value = match (use_v5, database) {
                (true, Some(db)) => BoltValue::Map(BTreeMap::from([(
                    "db".to_string(),
                    BoltValue::String(db.to_string()),
                )])),
                (true, None) => BoltValue::Map(BTreeMap::new()),
                (false, Some(db)) => BoltValue::String(db.to_string()),
                (false, None) => BoltValue::Null,
            };
            match self
                .exchange(0x66, &[context.clone(), bookmarks.clone(), db_value])
                .await
            {
                Ok(meta) => {
                    tracing::debug!(
                        "ROUTE (v5form={}) SUCCESS keys: {:?}",
                        use_v5,
                        meta.keys().collect::<Vec<_>>()
                    );
                    return parse_routing_table(&meta);
                }
                Err(e) => {
                    tracing::debug!("ROUTE (v5form={}) failed: {}", use_v5, e);
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            MigrateError::Protocol("ROUTE failed with no response".to_string())
        }))
    }

    pub async fn begin(&mut self, database: Option<&str>) -> Result<(), MigrateError> {
        let mut extra = BTreeMap::new();
        if let Some(db) = database {
            extra.insert("db".to_string(), BoltValue::String(db.to_string()));
        }
        self.exchange(0x11, &[BoltValue::Map(extra)]).await?;
        Ok(())
    }

    pub async fn commit(&mut self) -> Result<(), MigrateError> {
        self.exchange(0x12, &[]).await?;
        Ok(())
    }

    pub async fn rollback(&mut self) -> Result<(), MigrateError> {
        self.exchange(0x13, &[]).await?;
        Ok(())
    }

    pub async fn goodbye(&mut self) {
        let _ = self.send_message(0x02, &[]).await;
    }

    pub async fn run_count(&mut self, query: &str) -> Result<i64, MigrateError> {
        let (rows, _) = self.run(query, BTreeMap::new()).await?;
        let count = rows
            .first()
            .and_then(|row| row.first())
            .and_then(|v| v.as_int())
            .ok_or_else(|| {
                MigrateError::Protocol(format!("Count query did not return an integer: {}", query))
            })?;
        Ok(count)
    }
}

fn failure_details(fields: &[BoltValue]) -> (String, String) {
    match fields {
        [BoltValue::Map(meta)] => {
            let code = meta
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("Neo.Unknown")
                .to_string();
            let message = meta
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown server error")
                .to_string();
            (code, message)
        }
        _ => (
            "Neo.Unknown".to_string(),
            "malformed FAILURE message".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_routing_table_roles() {
        use std::collections::BTreeMap;
        let mut meta = BTreeMap::new();
        meta.insert("ttl".to_string(), BoltValue::Int(300));
        meta.insert("db".to_string(), BoltValue::String("neo4j".to_string()));
        let entry = |role: &str, addresses: Vec<&str>| {
            let mut map = BTreeMap::new();
            map.insert("role".to_string(), BoltValue::String(role.to_string()));
            map.insert(
                "addresses".to_string(),
                BoltValue::List(
                    addresses
                        .into_iter()
                        .map(|a| BoltValue::String(a.to_string()))
                        .collect(),
                ),
            );
            BoltValue::Map(map)
        };
        meta.insert(
            "servers".to_string(),
            BoltValue::List(vec![
                entry("WRITE", vec!["w1:7687"]),
                entry("READ", vec!["r1:7687", "r2:7687"]),
                entry("ROUTE", vec!["rt1:7687"]),
            ]),
        );
        let table = parse_routing_table(&meta).unwrap();
        assert_eq!(table.ttl_secs, 300);
        assert_eq!(table.database.as_deref(), Some("neo4j"));
        assert_eq!(table.writers, vec!["w1:7687".to_string()]);
        assert_eq!(table.readers.len(), 2);
        assert_eq!(table.routers, vec!["rt1:7687".to_string()]);

        // v5 role spellings are accepted too.
        if let BoltValue::List(servers) = meta.get_mut("servers").unwrap() {
            *servers = vec![entry("WRITER", vec!["w2:7687"])];
        }
        let table = parse_routing_table(&meta).unwrap();
        assert_eq!(table.writers, vec!["w2:7687".to_string()]);

        // Bolt 5 nests the same table under `rt`.
        let mut nested = BTreeMap::new();
        nested.insert(
            "rt".to_string(),
            BoltValue::Map(meta.clone()),
        );
        let table = parse_routing_table(&nested).unwrap();
        assert_eq!(table.writers, vec!["w2:7687".to_string()]);

        // Missing server list is malformed.
        meta.remove("servers");
        assert!(parse_routing_table(&meta).is_err());
    }

    #[test]
    fn test_parse_host_port() {
        assert_eq!(
            parse_host_port("db.example.com:7687"),
            ("db.example.com".to_string(), 7687)
        );
        assert_eq!(
            parse_host_port("db.example.com"),
            ("db.example.com".to_string(), 7687)
        );
        assert_eq!(
            parse_host_port("127.0.0.1:notaport"),
            ("127.0.0.1:notaport".to_string(), 7687)
        );
    }

    #[test]
    fn test_bolt_uri_parse() {
        let uri = BoltUri::parse("neo4j+s://abc.databases.neo4j.io:7687").unwrap();
        assert_eq!(uri.scheme, "neo4j+s");
        assert!(uri.use_tls);
        assert_eq!(uri.host, "abc.databases.neo4j.io");
        assert_eq!(uri.port, 7687);
        assert!(uri.is_routing_scheme());

        let uri = BoltUri::parse("bolt://localhost").unwrap();
        assert!(!uri.use_tls);
        assert_eq!(uri.port, 7687);
        assert!(!uri.is_routing_scheme());
    }

    #[test]
    fn test_bolt_uri_rejects_credentials_and_ssc() {
        assert!(BoltUri::parse("bolt://neo4j:secret@localhost").is_err());
        assert!(BoltUri::parse("neo4j+ssc://host").is_err());
        assert!(BoltUri::parse("http://localhost:7474").is_err());
        assert!(BoltUri::parse("not a uri").is_err());
    }

    #[test]
    fn test_version_encoding() {
        assert_eq!(version_to_u32(4, 4).to_be_bytes(), [0, 0, 4, 4]);
        assert_eq!(version_to_u32(3, 0).to_be_bytes(), [0, 0, 0, 3]);
    }

    #[test]
    fn test_chunk_message_framing() {
        let framed = chunk_message(&[1, 2, 3]);
        assert_eq!(framed, vec![0x00, 0x03, 1, 2, 3, 0x00, 0x00]);

        let big = vec![0xABu8; MAX_CHUNK_LEN + 10];
        let framed = chunk_message(&big);
        assert_eq!(&framed[0..2], &[0x40, 0x00]);
        assert!(framed.ends_with(&[0x00, 0x00]));
    }

    #[test]
    fn test_server_major() {
        assert_eq!(server_major("Neo4j/5.27-aura"), 5);
        assert_eq!(server_major("Neo4j/4.4.0"), 4);
        assert_eq!(server_major(""), 0);
        assert_eq!(server_major("not-a-version"), 0);
    }

    /// ROUTE tries the server-version form first, then falls back to the
    /// other form on the same connection: the fake 5.x server rejects the
    /// v5 map form and the client still routes via the v4 string form.
    #[tokio::test]
    async fn test_route_falls_back_to_other_form() {
        use crate::packstream::{decode_message, encode_message, BoltValue as BV};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        async fn read_msg(stream: &mut tokio::net::TcpStream) -> (u8, Vec<BV>) {
            let mut payload = Vec::new();
            loop {
                let mut len = [0u8; 2];
                stream.read_exact(&mut len).await.unwrap();
                let n = u16::from_be_bytes(len) as usize;
                if n == 0 {
                    break;
                }
                let mut chunk = vec![0u8; n];
                stream.read_exact(&mut chunk).await.unwrap();
                payload.extend_from_slice(&chunk);
            }
            decode_message(&payload).unwrap()
        }

        async fn send_msg(stream: &mut tokio::net::TcpStream, tag: u8, fields: &[BV]) {
            let raw = encode_message(tag, fields).unwrap();
            stream
                .write_all(&crate::bolt::chunk_message(&raw))
                .await
                .unwrap();
        }

        fn map(pairs: &[(&str, BV)]) -> BV {
            BV::Map(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut hs = [0u8; 20];
            stream.read_exact(&mut hs).await.unwrap();
            stream
                .write_all(&0x00000404u32.to_be_bytes())
                .await
                .unwrap(); // Bolt 4.4

            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x01); // HELLO
            send_msg(
                &mut stream,
                0x70,
                &[map(&[(
                    "server",
                    BV::String("Neo4j/5.27-fake".to_string()),
                )])],
            )
            .await;

            // First ROUTE: v5 form first on a 5.x agent (db as a map).
            let (tag, fields) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x66);
            match fields.as_slice() {
                [BV::Map(_), BV::List(_), BV::Map(db)] => {
                    assert_eq!(db.get("db").and_then(|v| v.as_str()), Some("mydb"));
                }
                other => panic!("first ROUTE must use the v5 map form, got {:?}", other),
            }
            // Reject it like a server that only speaks the v4 form.
            send_msg(
                &mut stream,
                0x7F,
                &[map(&[
                    (
                        "code",
                        BV::String("Neo.ClientError.Request.Invalid".to_string()),
                    ),
                    ("message", BV::String("Expected STRING but got MAP".to_string())),
                ])],
            )
            .await;
            let (tag, _) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x0F); // RESET after failure
            send_msg(&mut stream, 0x70, &[map(&[])]).await;

            // Second ROUTE: v4 fallback (db as a bare string) succeeds.
            let (tag, fields) = read_msg(&mut stream).await;
            assert_eq!(tag, 0x66);
            match fields.as_slice() {
                [BV::Map(_), BV::List(_), BV::String(db)] => assert_eq!(db, "mydb"),
                other => panic!("second ROUTE must use the v4 string form, got {:?}", other),
            }
            let mut writer = BTreeMap::new();
            writer.insert("role".to_string(), BV::String("WRITE".to_string()));
            writer.insert(
                "addresses".to_string(),
                BV::List(vec![BV::String("w1:7687".to_string())]),
            );
            send_msg(
                &mut stream,
                0x70,
                &[map(&[
                    ("ttl", BV::Int(300)),
                    ("db", BV::String("mydb".to_string())),
                    ("servers", BV::List(vec![BV::Map(writer)])),
                ])],
            )
            .await;
        });

        let uri = BoltUri::parse(&format!("bolt://127.0.0.1:{}", port)).unwrap();
        let config = BoltConfig::new(uri, "neo4j".to_string(), "test".to_string());
        let mut client = BoltClient::connect(&config).await.unwrap();
        let table = client
            .route("frontdoor:7687", Some("mydb"))
            .await
            .expect("ROUTE fallback must succeed");
        assert_eq!(table.writers, vec!["w1:7687".to_string()]);
        server.abort();
    }

    #[test]
    fn test_hello_message_bytes() {
        // Struct header: 1 field, tag 0x01 (HELLO).
        let mut auth = BTreeMap::new();
        auth.insert("scheme".to_string(), BoltValue::String("basic".to_string()));
        let bytes = encode_message(0x01, &[BoltValue::Map(auth)]).unwrap();
        assert_eq!(&bytes[0..2], &[0xB1, 0x01]);
    }

    /// Live check against a real Neo4j/Aura server. Not part of the default
    /// suite: run explicitly with NEO4J_URI, NEO4J_USER and NEO4J_PASSWORD set
    /// while a server is reachable.
    #[tokio::test]
    #[ignore]
    async fn test_live_bolt_round_trip() {
        let uri = std::env::var("NEO4J_URI")
            .unwrap_or_else(|_| panic!("Set NEO4J_URI to run this live test"));
        let username = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
        let password = std::env::var("NEO4J_PASSWORD")
            .unwrap_or_else(|_| panic!("Set NEO4J_PASSWORD to run this live test"));

        let config = BoltConfig::new(BoltUri::parse(&uri).unwrap(), username, password);
        let mut client = BoltClient::connect(&config)
            .await
            .expect("Bolt connect failed");
        assert!(!client.server_agent.is_empty());

        let (rows, _) = client
            .run("RETURN 1 AS one", BTreeMap::new())
            .await
            .expect("Bolt query failed");
        assert_eq!(rows, vec![vec![BoltValue::Int(1)]]);
        client.goodbye().await;
    }
}
