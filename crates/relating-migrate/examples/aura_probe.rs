//! Temporary Aura diagnosis probe (deleted after use).
use relating_migrate::{BoltClient, BoltConfig, BoltUri};
use std::collections::BTreeMap;

fn load_dotenv(path: &str) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }
}

#[tokio::main]
async fn main() {
    load_dotenv(".env");
    let uri = std::env::var("AURA_URI").unwrap_or_default();
    let username = std::env::var("AURA_USERNAME").unwrap_or_default();
    let password = std::env::var("AURA_PASSWORD").unwrap_or_default();
    let database = std::env::var("AURA_DATABASE").unwrap_or_default();
    println!("URI: {}", uri);
    println!("USERNAME: {} (len={})", username, username.len());
    println!("PASSWORD: len={}", password.len());
    println!("DATABASE: {} (len={})", database, database.len());

    let parsed = BoltUri::parse(&uri).expect("parse uri");
    let mut config = BoltConfig::new(parsed, username.clone(), password.clone());

    // Step 1: connect without database, run RETURN 1
    let mut client = match BoltClient::connect(&config).await {
        Ok(c) => {
            println!(
                "CONNECT(no db): OK server='{}' bolt={}.{}",
                c.server_agent, c.negotiated_major, c.negotiated_minor
            );
            c
        }
        Err(e) => {
            println!("CONNECT(no db): FAIL {}", e);
            return;
        }
    };
    match client.run("RETURN 1 AS one", BTreeMap::new()).await {
        Ok((rows, _)) => println!("RETURN 1: OK rows={:?}", rows),
        Err(e) => println!("RETURN 1: FAIL {}", e),
    }

    // Step 2: what databases are visible?
    match client
        .run("SHOW DATABASES YIELD name", BTreeMap::new())
        .await
    {
        Ok((rows, _)) => println!("SHOW DATABASES: OK {:?}", rows),
        Err(e) => println!("SHOW DATABASES: FAIL {}", e),
    }

    // Step 3: begin with the configured database (what test_connection does)
    config.database = Some(database.clone());
    let mut client2 = BoltClient::connect(&config).await.expect("reconnect");
    match client2.begin(Some(&database)).await {
        Ok(()) => {
            println!("BEGIN(db='{}'): OK", database);
            let _ = client2.rollback().await;
        }
        Err(e) => println!("BEGIN(db='{}'): FAIL {}", database, e),
    }

    // Step 4: begin with neo4j
    match client2.begin(Some("neo4j")).await {
        Ok(()) => {
            println!("BEGIN(db='neo4j'): OK");
            let _ = client2.rollback().await;
        }
        Err(e) => println!("BEGIN(db='neo4j'): FAIL {}", e),
    }
}
