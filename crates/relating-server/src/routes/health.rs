use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};

pub async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "product": "Relating",
        "timestamp": Utc::now().to_rfc3339()
    }))
}
