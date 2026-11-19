use crate::error::MigrateError;
use crate::packstream::{decode_message, encode_message, BoltValue};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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

impl BoltClient {
    pub async fn connect(config: &BoltConfig) -> Result<Self, MigrateError> {
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

        let mut rows = Vec::new();
        loop {
            let (tag, fields) = self.recv_message().await?;
            match tag {
                0x71 => {
                    // RECORD
                    match fields.as_slice() {
                        [BoltValue::List(cols)] => rows.push(cols.clone()),
                        _ => {
                            return Err(MigrateError::Protocol(
                                "Malformed RECORD message".to_string(),
                            ))
                        }
                    }
                }
                0x70 => {
                    // SUCCESS
                    match fields.as_slice() {
                        [BoltValue::Map(meta)] => return Ok((rows, meta.clone())),
                        _ => {
                            return Err(MigrateError::Protocol(
                                "Malformed SUCCESS message".to_string(),
                            ))
                        }
                    }
                }
                0x7F => {
                    let (code, message) = failure_details(&fields);
                    self.reset_after_failure().await;
                    if code.contains("Unauthorized") || code.contains("Authentication") {
                        return Err(MigrateError::Auth(message));
                    }
                    return Err(MigrateError::Protocol(format!("{}: {}", code, message)));
                }
                0x7E => {
                    return Err(MigrateError::Protocol(
                        "Server ignored the request (previous failure)".to_string(),
                    ))
                }
                _ => {
                    return Err(MigrateError::Protocol(format!(
                        "Unexpected response message {:#04x}",
                        tag
                    )))
                }
            }
        }
    }

    /// RUN + PULL that also returns the result column names.
    ///
    /// Like [`BoltClient::run`] this pipelines RUN and PULL, then reads every
    /// response: the RUN SUCCESS (which carries `fields` on real servers) is
    /// captured and reading continues through RECORDs to the final SUCCESS.
    /// Proxy mode uses it so Aura results carry the same shaped columns as
    /// local Relate results. Existing callers of `run` are unaffected.
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
