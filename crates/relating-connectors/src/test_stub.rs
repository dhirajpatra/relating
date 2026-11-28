//! Raw-socket JSON stub for connector integration tests.
//!
//! Test-only helper (no HTTP framework dependency): serves canned
//! `(status, headers, body)` tuples by path prefix over plain HTTP/1.0 on
//! loopback. Supports scripted failure-then-success sequences so retry and
//! pagination paths run end to end without mocks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
pub struct StubRoute {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl StubRoute {
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            reason: "OK".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: body.to_string(),
        }
    }

    pub fn status(status: u16, reason: &str, body: &str) -> Self {
        Self {
            status,
            reason: reason.to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: body.to_string(),
        }
    }
}

/// Scripted stub: each path serves its route queue in order (repeating the
/// last), so tests script `503, 503, 200` retry sequences deterministically.
#[derive(Clone)]
pub struct StubScript {
    routes: Arc<Mutex<HashMap<String, Vec<StubRoute>>>>,
    hits: Arc<Mutex<Vec<String>>>,
}

impl StubScript {
    pub fn new(routes: Vec<(String, Vec<StubRoute>)>) -> Self {
        Self {
            routes: Arc::new(Mutex::new(routes.into_iter().collect())),
            hits: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }

    fn next_route(&self, path: &str) -> StubRoute {
        self.hits.lock().unwrap().push(path.to_string());
        let mut routes = self.routes.lock().unwrap();
        // Longest-prefix match so `/items?page=2` hits the `/items` script.
        let key = routes
            .keys()
            .filter(|prefix| path.starts_with(prefix.as_str()))
            .max_by_key(|prefix| prefix.len())
            .cloned();
        match key.and_then(|k| routes.get_mut(&k)) {
            Some(queue) if queue.len() > 1 => queue.remove(0),
            Some(queue) => queue[0].clone(),
            None => StubRoute::status(404, "Not Found", r#"{"error":"no route"}"#),
        }
    }
}

async fn handle_connection(mut stream: tokio::net::TcpStream, script: StubScript) {
    let mut buf = vec![0u8; 8192];
    let Ok(n) = stream.read(&mut buf).await else {
        return;
    };
    let head = String::from_utf8_lossy(&buf[..n]);
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    let route = script.next_route(&path);
    let mut response = format!(
        "HTTP/1.0 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        route.status,
        route.reason,
        route.body.len()
    );
    for (name, value) in &route.headers {
        response.push_str(&format!("{}: {}\r\n", name, value));
    }
    response.push_str("\r\n");
    response.push_str(&route.body);
    let _ = stream.write_all(response.as_bytes()).await;
}

/// Start the stub; returns the base URL, the script (for hit assertions),
/// and the server task (abort it when done).
pub async fn start_json_stub(
    routes: Vec<(String, Vec<StubRoute>)>,
) -> (String, StubScript, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = StubScript::new(routes);
    let worker = script.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let script = worker.clone();
            tokio::spawn(handle_connection(stream, script));
        }
    });
    (format!("http://127.0.0.1:{}", port), script, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_serves_queued_routes_in_order() {
        let (base, script, server) = start_json_stub(vec![(
            "/items".to_string(),
            vec![
                StubRoute::status(503, "Busy", "{}"),
                StubRoute::json(r#"{"ok":true}"#),
            ],
        )])
        .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let first = client.get(format!("{}/items", base)).send().await.unwrap();
        assert_eq!(first.status(), 503);
        let second = client
            .get(format!("{}/items?page=2", base))
            .send()
            .await
            .unwrap();
        assert_eq!(second.status(), 200);
        assert_eq!(script.hits().len(), 2);
        server.abort();
    }
}
