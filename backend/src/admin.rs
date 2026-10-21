//! Account administration uses the same session, origin checks and password hashing as login.
use crate::{
    AppState, User,
    auth::{self, Session},
    error::{AppError, Result},
    now,
    websocket::Event,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountInput {
    username: String,
    role: String,
    #[serde(default)]
    password: String,
}
fn validate(i: &AccountInput, creating: bool) -> Result<String> {
    if !["admin", "member"].contains(&i.role.as_str()) {
        return Err(AppError::bad("Role deve ser admin ou member"));
    }
    if creating || !i.password.is_empty() {
        auth::validate_password(&i.password)?;
    }
    auth::validate_username(&i.username)
}
async fn find(s: &AppState, id: &str) -> Result<User> {
    sqlx::query_as("SELECT id,username,role FROM users WHERE id=?")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(AppError::missing)
}
// Serialize role changes/deletions and recheck the actor after obtaining SQLite's write lock.
async fn transaction(
    s: &AppState,
    actor: &Session,
) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
    auth::admin(actor)?;
    let mut tx = s.db.begin_with("BEGIN IMMEDIATE").await?;
    let allowed: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin')")
            .bind(&actor.user.id)
            .fetch_one(&mut *tx)
            .await?;
    if !allowed {
        return Err(AppError::forbidden());
    }
    Ok(tx)
}
async fn protect_last(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, id: &str) -> Result<()> {
    let last: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND role='admin') AND (SELECT COUNT(*) FROM users WHERE role='admin')=1").bind(id).fetch_one(&mut **tx).await?;
    if last {
        return Err(AppError::bad(
            "Não é possível remover ou rebaixar o último administrador",
        ));
    }
    Ok(())
}
pub async fn create(
    State(s): State<AppState>,
    actor: Session,
    Json(i): Json<AccountInput>,
) -> Result<Json<User>> {
    auth::admin(&actor)?;
    s.rate(format!("accounts:{}", actor.user.id), 20).await?;
    let username = validate(&i, true)?;
    let hash = auth::hash_password(&s, i.password).await?;
    let mut tx = transaction(&s, &actor).await?;
    let user = User {
        id: uuid::Uuid::new_v4().to_string(),
        username,
        role: i.role,
    };
    sqlx::query("INSERT INTO users(id,username,password_hash,role,created_at) VALUES(?,?,?,?,?)")
        .bind(&user.id)
        .bind(&user.username)
        .bind(hash)
        .bind(&user.role)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    s.emit(Event::UsersChanged);
    Ok(Json(user))
}
pub async fn update(
    State(s): State<AppState>,
    actor: Session,
    Path(id): Path<String>,
    Json(i): Json<AccountInput>,
) -> Result<Json<User>> {
    auth::admin(&actor)?;
    s.rate(format!("accounts:{}", actor.user.id), 20).await?;
    let username = validate(&i, false)?;
    let hash = if i.password.is_empty() {
        None
    } else {
        Some(auth::hash_password(&s, i.password).await?)
    };
    let mut tx = transaction(&s, &actor).await?;
    if i.role == "member" {
        protect_last(&mut tx, &id).await?;
    }
    let result = sqlx::query(
        "UPDATE users SET username=?,role=?,password_hash=COALESCE(?,password_hash) WHERE id=?",
    )
    .bind(&username)
    .bind(&i.role)
    .bind(&hash)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::missing());
    }
    let revoked: Vec<String> = if hash.is_some() {
        let sessions = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=?")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id=?")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        sessions
    } else {
        vec![]
    };
    tx.commit().await?;
    for session_hash in revoked {
        s.emit(Event::SessionRevoked { session_hash });
    }
    if hash.is_some() {
        remove_voice(&s, &id).await;
    }
    if let Some(lease) = s.voice.lock().await.get_mut(&id) {
        lease.name = username.clone();
    }
    s.emit(Event::UsersChanged);
    Ok(Json(User {
        id,
        username,
        role: i.role,
    }))
}
pub async fn delete(
    State(s): State<AppState>,
    actor: Session,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let mut tx = transaction(&s, &actor).await?;
    protect_last(&mut tx, &id).await?;
    let sessions: Vec<String> =
        sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=?")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
    // Invitations and messages have restrictive foreign keys. Account deletion removes these records too.
    sqlx::query("DELETE FROM invites WHERE created_by=? OR used_by=?")
        .bind(&id)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM messages WHERE user_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let result = sqlx::query("DELETE FROM users WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::missing());
    }
    tx.commit().await?;
    for session_hash in sessions {
        s.emit(Event::SessionRevoked { session_hash });
    }
    remove_voice(&s, &id).await;
    s.emit(Event::UsersChanged);
    Ok(Json(json!({"ok":true})))
}
async fn remove_voice(s: &AppState, id: &str) {
    let removed = s.voice.lock().await.remove(id);
    if let Some(lease) = removed {
        s.media.remove_lease(&lease.lease).await;
        s.emit(Event::VoiceRemoved { user_id: id.into() });
        s.emit(Event::PresenceChanged);
    }
}
pub async fn disconnect(
    State(s): State<AppState>,
    actor: Session,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    auth::admin(&actor)?;
    find(&s, &id).await?;
    remove_voice(&s, &id).await;
    Ok(Json(json!({"ok":true})))
}
