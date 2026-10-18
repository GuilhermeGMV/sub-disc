use crate::{
    AppState,
    auth::{Session, admin},
    error::{AppError, Result},
    now,
    websocket::Event,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_at: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelInput {
    pub name: String,
    pub kind: String,
}
fn validate(i: &ChannelInput) -> Result<()> {
    if i.name.trim().is_empty()
        || i.name.chars().count() > 48
        || i.name.chars().any(char::is_control)
    {
        return Err(AppError::bad("Nome de canal: 1–48 caracteres"));
    }
    if !["text", "voice"].contains(&i.kind.as_str()) {
        return Err(AppError::bad("Tipo de canal inválido"));
    }
    Ok(())
}
pub async fn all(s: &AppState) -> Result<Vec<Channel>> {
    Ok(
        sqlx::query_as("SELECT * FROM channels ORDER BY created_at,id")
            .fetch_all(&s.db)
            .await?,
    )
}
pub async fn list(State(s): State<AppState>, _: Session) -> Result<Json<Vec<Channel>>> {
    Ok(Json(all(&s).await?))
}
pub async fn find(s: &AppState, id: &str) -> Result<Channel> {
    sqlx::query_as("SELECT * FROM channels WHERE id=?")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(AppError::missing)
}
pub async fn create(
    State(s): State<AppState>,
    user: Session,
    Json(i): Json<ChannelInput>,
) -> Result<Json<Channel>> {
    admin(&user)?;
    validate(&i)?;
    s.rate(format!("channels:{}", user.user.id), 20).await?;
    let c = Channel {
        id: uuid::Uuid::new_v4().to_string(),
        name: i.name.trim().into(),
        kind: i.kind,
        created_at: now(),
    };
    sqlx::query("INSERT INTO channels VALUES(?,?,?,?)")
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.kind)
        .bind(c.created_at)
        .execute(&s.db)
        .await?;
    s.emit(Event::ChannelsChanged);
    Ok(Json(c))
}
pub async fn update(
    State(s): State<AppState>,
    user: Session,
    Path(id): Path<String>,
    Json(i): Json<ChannelInput>,
) -> Result<Json<Channel>> {
    admin(&user)?;
    validate(&i)?;
    let c = find(&s, &id).await?;
    if c.kind != i.kind {
        return Err(AppError::bad("O tipo do canal não pode ser alterado"));
    }
    sqlx::query("UPDATE channels SET name=? WHERE id=?")
        .bind(i.name.trim())
        .bind(&id)
        .execute(&s.db)
        .await?;
    s.emit(Event::ChannelsChanged);
    Ok(Json(find(&s, &id).await?))
}
pub async fn delete(
    State(s): State<AppState>,
    user: Session,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    admin(&user)?;
    let c = find(&s, &id).await?;
    if c.kind == "voice" {
        crate::media::delete_room(&s, &id).await?;
    }
    sqlx::query("DELETE FROM channels WHERE id=?")
        .bind(&id)
        .execute(&s.db)
        .await?;
    s.emit(Event::ChannelsChanged);
    Ok(Json(serde_json::json!({"ok":true})))
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatMessage {
    pub id: i64,
    pub channel_id: String,
    pub user_id: String,
    pub username: String,
    pub content: String,
    pub created_at: i64,
}
#[derive(Deserialize)]
pub struct Page {
    before: Option<i64>,
    limit: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInput {
    pub content: String,
}
pub async fn history(
    State(s): State<AppState>,
    _: Session,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<ChatMessage>>> {
    if find(&s, &id).await?.kind != "text" {
        return Err(AppError::bad("Este canal não é de texto"));
    }
    let limit = page.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(AppError::bad("Limite: 1–100"));
    }
    let mut messages:Vec<ChatMessage>=sqlx::query_as("SELECT m.*,u.username FROM messages m JOIN users u ON u.id=m.user_id WHERE m.channel_id=? AND m.id<? ORDER BY m.id DESC LIMIT ?").bind(id).bind(page.before.unwrap_or(i64::MAX)).bind(limit).fetch_all(&s.db).await?;
    messages.reverse();
    Ok(Json(messages))
}
pub async fn send(
    State(s): State<AppState>,
    user: Session,
    Path(id): Path<String>,
    Json(i): Json<MessageInput>,
) -> Result<Json<ChatMessage>> {
    s.rate(format!("message:{}", user.user.id), 60).await?;
    let content = i.content.trim();
    if content.is_empty() || content.chars().count() > 4000 {
        return Err(AppError::bad("Mensagem: 1–4000 caracteres"));
    }
    if find(&s, &id).await?.kind != "text" {
        return Err(AppError::bad("Este canal não é de texto"));
    }
    let created_at = now();
    let row =
        sqlx::query("INSERT INTO messages(channel_id,user_id,content,created_at) VALUES(?,?,?,?)")
            .bind(&id)
            .bind(&user.user.id)
            .bind(content)
            .bind(created_at)
            .execute(&s.db)
            .await?;
    let msg = ChatMessage {
        id: row.last_insert_rowid(),
        channel_id: id,
        user_id: user.user.id,
        username: user.user.username,
        content: content.into(),
        created_at,
    };
    s.emit(Event::Message {
        message: msg.clone(),
    });
    Ok(Json(msg))
}
