use chrono::{DateTime, Utc};
use relating_core::{
    Chunk, ChunkId, EmbeddingProfile, EmbeddingProfileId, ProjectId, RelatingError, SourceId,
};
use sqlx::{Row, SqlitePool};

pub fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

pub fn decode_embedding(bytes: &[u8]) -> Result<Vec<f32>, RelatingError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(RelatingError::Internal(
            "Corrupt embedding blob: byte length is not a multiple of 4".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for arr in bytes.as_chunks::<4>().0 {
        out.push(f32::from_le_bytes(*arr));
    }
    Ok(out)
}

pub async fn insert_embedding_profile(
    pool: &SqlitePool,
    profile: &EmbeddingProfile,
) -> Result<(), RelatingError> {
    sqlx::query(
        r#"
        INSERT INTO embedding_profiles (id, project_id, name, provider, model, model_version, dimensions, modality, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(profile.id.to_string())
    .bind(profile.project_id.to_string())
    .bind(&profile.name)
    .bind(&profile.provider)
    .bind(&profile.model)
    .bind(profile.model_version.clone())
    .bind(profile.dimensions as i64)
    .bind(&profile.modality)
    .bind(profile.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert embedding profile: {}", e)))?;

    Ok(())
}

fn map_profile_row(row: sqlx::sqlite::SqliteRow) -> Result<EmbeddingProfile, RelatingError> {
    let id_str: String = row.get("id");
    let project_id_str: String = row.get("project_id");
    let name: String = row.get("name");
    let provider: String = row.get("provider");
    let model: String = row.get("model");
    let model_version: Option<String> = row.get("model_version");
    let dimensions: i64 = row.get("dimensions");
    let modality: String = row.get("modality");
    let created_at_str: String = row.get("created_at");

    let id = EmbeddingProfileId::parse(&id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid EmbeddingProfile UUID: {}", e)))?;
    let project_id = ProjectId::parse(&project_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);

    Ok(EmbeddingProfile {
        id,
        project_id,
        name,
        provider,
        model,
        model_version,
        dimensions: dimensions as usize,
        modality,
        created_at,
    })
}

pub async fn get_embedding_profile(
    pool: &SqlitePool,
    id: &EmbeddingProfileId,
) -> Result<Option<EmbeddingProfile>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, name, provider, model, model_version, dimensions, modality, created_at
        FROM embedding_profiles
        WHERE id = ?
        "#,
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch embedding profile: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_profile_row(row)?)),
        None => Ok(None),
    }
}

pub async fn get_embedding_profile_by_name(
    pool: &SqlitePool,
    project_id: &ProjectId,
    name: &str,
) -> Result<Option<EmbeddingProfile>, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, name, provider, model, model_version, dimensions, modality, created_at
        FROM embedding_profiles
        WHERE project_id = ? AND name = ?
        "#,
    )
    .bind(project_id.to_string())
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to fetch embedding profile: {}", e)))?;

    match row {
        Some(row) => Ok(Some(map_profile_row(row)?)),
        None => Ok(None),
    }
}

pub async fn list_embedding_profiles(
    pool: &SqlitePool,
    project_id: &ProjectId,
) -> Result<Vec<EmbeddingProfile>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, name, provider, model, model_version, dimensions, modality, created_at
        FROM embedding_profiles
        WHERE project_id = ?
        ORDER BY created_at ASC
        "#,
    )
    .bind(project_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list embedding profiles: {}", e)))?;

    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
        profiles.push(map_profile_row(row)?);
    }
    Ok(profiles)
}

pub async fn insert_chunk(pool: &SqlitePool, chunk: &Chunk) -> Result<(), RelatingError> {
    let metadata_json = serde_json::to_string(&chunk.metadata).map_err(|e| {
        RelatingError::Internal(format!("Failed to serialize chunk metadata: {}", e))
    })?;

    sqlx::query(
        r#"
        INSERT INTO chunks (id, project_id, profile_id, source_id, ordinal, content, content_hash, metadata, embedding, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, content_hash) DO NOTHING
        "#,
    )
    .bind(chunk.id.to_string())
    .bind(chunk.project_id.to_string())
    .bind(chunk.profile_id.to_string())
    .bind(chunk.source_id.to_string())
    .bind(chunk.ordinal as i64)
    .bind(&chunk.content)
    .bind(&chunk.content_hash)
    .bind(metadata_json)
    .bind(encode_embedding(&chunk.embedding))
    .bind(chunk.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to insert chunk: {}", e)))?;

    Ok(())
}

pub async fn chunk_exists(
    pool: &SqlitePool,
    profile_id: &EmbeddingProfileId,
    content_hash: &str,
) -> Result<bool, RelatingError> {
    let row = sqlx::query(
        r#"
        SELECT id FROM chunks WHERE profile_id = ? AND content_hash = ? LIMIT 1
        "#,
    )
    .bind(profile_id.to_string())
    .bind(content_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to check chunk existence: {}", e)))?;

    Ok(row.is_some())
}

fn map_chunk_row(row: sqlx::sqlite::SqliteRow) -> Result<Chunk, RelatingError> {
    let id_str: String = row.get("id");
    let project_id_str: String = row.get("project_id");
    let profile_id_str: String = row.get("profile_id");
    let source_id_str: String = row.get("source_id");
    let ordinal: i64 = row.get("ordinal");
    let content: String = row.get("content");
    let content_hash: String = row.get("content_hash");
    let metadata_str: String = row.get("metadata");
    let embedding_bytes: Vec<u8> = row.get("embedding");
    let created_at_str: String = row.get("created_at");

    let id = ChunkId::parse(&id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Chunk UUID: {}", e)))?;
    let project_id = ProjectId::parse(&project_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Project UUID: {}", e)))?;
    let profile_id = EmbeddingProfileId::parse(&profile_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid EmbeddingProfile UUID: {}", e)))?;
    let source_id = SourceId::parse(&source_id_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid Source UUID: {}", e)))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| RelatingError::Internal(format!("Invalid timestamp: {}", e)))?
        .with_timezone(&Utc);

    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null);
    let embedding = decode_embedding(&embedding_bytes)?;

    Ok(Chunk {
        id,
        project_id,
        profile_id,
        source_id,
        ordinal: ordinal as u32,
        content,
        content_hash,
        metadata,
        embedding,
        created_at,
    })
}

pub async fn list_chunks_for_profile(
    pool: &SqlitePool,
    profile_id: &EmbeddingProfileId,
) -> Result<Vec<Chunk>, RelatingError> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, profile_id, source_id, ordinal, content, content_hash, metadata, embedding, created_at
        FROM chunks
        WHERE profile_id = ?
        ORDER BY ordinal ASC
        "#,
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| RelatingError::Database(format!("Failed to list chunks: {}", e)))?;

    let mut chunks = Vec::with_capacity(rows.len());
    for row in rows {
        chunks.push(map_chunk_row(row)?);
    }
    Ok(chunks)
}

pub async fn count_chunks_for_profile(
    pool: &SqlitePool,
    profile_id: &EmbeddingProfileId,
) -> Result<u64, RelatingError> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM chunks WHERE profile_id = ?")
        .bind(profile_id.to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to count chunks: {}", e)))?;

    let count: i64 = row.get("c");
    Ok(count as u64)
}

pub async fn delete_chunks_for_source(
    pool: &SqlitePool,
    source_id: &SourceId,
) -> Result<u64, RelatingError> {
    let result = sqlx::query("DELETE FROM chunks WHERE source_id = ?")
        .bind(source_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| RelatingError::Database(format!("Failed to delete chunks: {}", e)))?;

    Ok(result.rows_affected())
}
