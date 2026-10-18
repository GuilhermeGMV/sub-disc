use crate::{
    AppState, User,
    auth::Session,
    channels::{self, Channel},
    error::Result,
    media::Lease,
};
use axum::{Json, extract::State};
use serde::Serialize;
#[derive(Serialize)]
pub struct Snapshot {
    pub users: Vec<User>,
    pub channels: Vec<Channel>,
    pub online: Vec<String>,
    pub voice: Vec<Lease>,
}
pub async fn health(State(s): State<AppState>) -> Result<Json<serde_json::Value>> {
    sqlx::query("SELECT 1").execute(&s.db).await?;
    Ok(Json(
        serde_json::json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}),
    ))
}
pub async fn users(State(s): State<AppState>, _: Session) -> Result<Json<Vec<User>>> {
    Ok(Json(
        sqlx::query_as("SELECT id,username,role FROM users ORDER BY username")
            .fetch_all(&s.db)
            .await?,
    ))
}
pub async fn state(s: &AppState) -> Result<Snapshot> {
    Ok(Snapshot {
        users: sqlx::query_as("SELECT id,username,role FROM users ORDER BY username")
            .fetch_all(&s.db)
            .await?,
        channels: channels::all(s).await?,
        online: s.online.lock().await.keys().cloned().collect(),
        voice: s
            .voice
            .lock()
            .await
            .values()
            .filter(|l| l.connected)
            .cloned()
            .collect(),
    })
}
pub async fn snapshot(State(s): State<AppState>, _: Session) -> Result<Json<Snapshot>> {
    Ok(Json(state(&s).await?))
}
