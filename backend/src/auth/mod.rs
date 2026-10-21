use crate::{
    AppState, User,
    error::{AppError, Result},
    now,
    websocket::Event,
};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
/// Validade da sessão, contada a partir do último uso (ver `authenticate`).
const SESSION_TTL: i64 = 30 * 86400;
#[derive(Clone)]
pub struct Session {
    pub user: User,
    pub hash: String,
    raw: String,
    expires_at: i64,
}
impl FromRequestParts<AppState> for Session {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self> {
        authenticate(&parts.headers, s).await
    }
}
pub fn digest(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}
pub fn token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub async fn authenticate(headers: &HeaderMap, s: &AppState) -> Result<Session> {
    let raw = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';')
                .find_map(|pair| pair.trim().strip_prefix("subdisc_session="))
        })
        .ok_or_else(AppError::unauthorized)?;
    if raw.len() != 64 {
        return Err(AppError::unauthorized());
    }
    let hash = digest(raw);
    let (id,username,role,mut expires_at)=sqlx::query_as::<_,(String,String,String,i64)>("SELECT u.id,u.username,u.role,s.expires_at FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.token_hash=? AND s.expires_at>?").bind(&hash).bind(now()).fetch_optional(&s.db).await?.ok_or_else(AppError::unauthorized)?;
    // Quem usa o app não precisa entrar de novo: a sessão só expira depois de
    // 30 dias sem uso. Grava no máximo uma vez a cada 15 dias por sessão.
    if expires_at - now() < SESSION_TTL / 2 {
        expires_at = now() + SESSION_TTL;
        sqlx::query("UPDATE sessions SET expires_at=? WHERE token_hash=?")
            .bind(expires_at)
            .bind(&hash)
            .execute(&s.db)
            .await?;
    }
    Ok(Session {
        user: User { id, username, role },
        hash,
        raw: raw.to_owned(),
        expires_at,
    })
}
pub fn admin(s: &Session) -> Result<()> {
    if s.user.role != "admin" {
        Err(AppError::forbidden())
    } else {
        Ok(())
    }
}
fn cookie(s: &AppState, raw: &str, age: i64) -> String {
    format!(
        "subdisc_session={raw}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}{}",
        if s.config.secure { "; Secure" } else { "" }
    )
}
async fn session_response(s: &AppState, user: User) -> Result<(HeaderMap, Json<User>)> {
    let raw = token();
    sqlx::query("DELETE FROM sessions WHERE expires_at<=?")
        .bind(now())
        .execute(&s.db)
        .await?;
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES(?,?,?)")
        .bind(digest(&raw))
        .bind(&user.id)
        .bind(now() + SESSION_TTL)
        .execute(&s.db)
        .await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        cookie(s, &raw, SESSION_TTL)
            .parse()
            .map_err(|_| AppError::internal())?,
    );
    Ok((headers, Json(user)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    invite: String,
    #[serde(default)]
    setup_key: String,
}
fn validate(c: &Credentials) -> Result<String> {
    let name = c.username.trim().to_lowercase();
    if !(3..=32).contains(&name.len())
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(AppError::bad(
            "Usuário: 3–32 letras sem acentos, números, _ ou -",
        ));
    }
    if !(12..=128).contains(&c.password.len()) {
        return Err(AppError::bad("Senha: entre 12 e 128 bytes"));
    }
    Ok(name)
}
async fn hash_password(s: &AppState, password: String) -> Result<String> {
    let permit = s.auth_slots.clone().try_acquire_owned().map_err(|_| {
        AppError(
            StatusCode::TOO_MANY_REQUESTS,
            "Servidor ocupado. Tente novamente.",
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|v| v.to_string())
            .map_err(|_| AppError::internal())
    })
    .await
    .map_err(|_| AppError::internal())?
}
pub async fn setup_status(State(s): State<AppState>) -> Result<Json<Value>> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&s.db)
        .await?;
    Ok(Json(json!({"needs_setup":count==0})))
}
pub async fn setup(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(c): Json<Credentials>,
) -> Result<(HeaderMap, Json<User>)> {
    s.rate(format!("auth:{}", peer.ip()), 60).await?;
    s.rate(
        format!("auth-name:{}", c.username.trim().to_lowercase()),
        10,
    )
    .await?;
    if digest(&c.setup_key) != digest(&s.config.setup_key) {
        return Err(AppError::forbidden());
    }
    let username = validate(&c)?;
    let hash = hash_password(&s, c.password).await?;
    let mut tx = s.db.begin_with("BEGIN IMMEDIATE").await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if count != 0 {
        return Err(AppError(StatusCode::CONFLICT, "Instância já configurada"));
    }
    let user = User {
        id: uuid::Uuid::new_v4().to_string(),
        username,
        role: "admin".into(),
    };
    sqlx::query("INSERT INTO users VALUES(?,?,?,?,?)")
        .bind(&user.id)
        .bind(&user.username)
        .bind(hash)
        .bind(&user.role)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    for (name, kind) in [
        ("geral", "text"),
        ("sala de estar", "voice"),
        ("jogando juntos", "voice"),
    ] {
        sqlx::query("INSERT INTO channels VALUES(?,?,?,?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(name)
            .bind(kind)
            .bind(now())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    session_response(&s, user).await
}
pub async fn register(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(c): Json<Credentials>,
) -> Result<(HeaderMap, Json<User>)> {
    s.rate(format!("auth:{}", peer.ip()), 60).await?;
    s.rate(
        format!("auth-name:{}", c.username.trim().to_lowercase()),
        10,
    )
    .await?;
    let username = validate(&c)?;
    if c.invite.len() != 64 {
        return Err(AppError::bad("Convite inválido"));
    }
    let hash = hash_password(&s, c.password).await?;
    let mut tx = s.db.begin_with("BEGIN IMMEDIATE").await?;
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM invites WHERE token_hash=? AND expires_at>? AND used_by IS NULL)").bind(digest(&c.invite)).bind(now()).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(AppError::bad("Convite expirado, utilizado ou inválido"));
    }
    let user = User {
        id: uuid::Uuid::new_v4().to_string(),
        username,
        role: "member".into(),
    };
    sqlx::query("INSERT INTO users VALUES(?,?,?,?,?)")
        .bind(&user.id)
        .bind(&user.username)
        .bind(hash)
        .bind(&user.role)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE invites SET used_by=? WHERE token_hash=?")
        .bind(&user.id)
        .bind(digest(&c.invite))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    s.emit(Event::UsersChanged);
    session_response(&s, user).await
}
pub async fn login(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(c): Json<Credentials>,
) -> Result<(HeaderMap, Json<User>)> {
    s.rate(format!("auth:{}", peer.ip()), 60).await?;
    s.rate(
        format!("auth-name:{}", c.username.trim().to_lowercase()),
        10,
    )
    .await?;
    let name = validate(&c)?;
    let row: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT id,username,role,password_hash FROM users WHERE username=?")
            .bind(name)
            .fetch_optional(&s.db)
            .await?;
    // A valid dummy hash keeps unknown-account attempts on the same expensive path.
    let dummy = "$argon2id$v=19$m=19456,t=2,p=1$c3ViZGlzYy1kdW1teS1zYWx0$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let hash = row
        .as_ref()
        .map(|r| r.3.clone())
        .unwrap_or_else(|| dummy.into());
    let permit = s
        .auth_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError(StatusCode::TOO_MANY_REQUESTS, "Servidor ocupado"))?;
    let valid = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        PasswordHash::new(&hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(c.password.as_bytes(), &hash)
                .is_ok()
        })
    })
    .await
    .map_err(|_| AppError::internal())?;
    match row {
        Some((id, username, role, _)) if valid => {
            session_response(&s, User { id, username, role }).await
        }
        _ => Err(AppError(
            StatusCode::UNAUTHORIZED,
            "Usuário ou senha incorretos",
        )),
    }
}
/// Também devolve o cookie com a validade atual, para o navegador acompanhar a
/// renovação feita em `authenticate`.
pub async fn me(State(s): State<AppState>, session: Session) -> Result<(HeaderMap, Json<User>)> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        cookie(&s, &session.raw, session.expires_at - now())
            .parse()
            .map_err(|_| AppError::internal())?,
    );
    Ok((headers, Json(session.user)))
}
pub async fn logout(
    State(s): State<AppState>,
    session: Session,
) -> Result<(HeaderMap, Json<Value>)> {
    sqlx::query("DELETE FROM sessions WHERE token_hash=?")
        .bind(&session.hash)
        .execute(&s.db)
        .await?;
    if media_logout(&s, &session).await.is_err() {
        tracing::warn!("Session revoked while SFU unavailable");
    }
    s.emit(Event::SessionRevoked {
        session_hash: session.hash,
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        cookie(&s, "", 0)
            .parse()
            .map_err(|_| AppError::internal())?,
    );
    Ok((headers, Json(json!({"ok":true}))))
}
async fn media_logout(s: &AppState, session: &Session) -> Result<()> {
    crate::media::release(s, session, None).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteInput {
    pub hours: i64,
}
pub async fn invite(
    State(s): State<AppState>,
    session: Session,
    Json(input): Json<InviteInput>,
) -> Result<Json<Value>> {
    admin(&session)?;
    s.rate(format!("invites:{}", session.user.id), 10).await?;
    if !(1..=168).contains(&input.hours) {
        return Err(AppError::bad("Validade: de 1 a 168 horas"));
    }
    let raw = token();
    let expires_at = now() + input.hours * 3600;
    sqlx::query("INSERT INTO invites(token_hash,created_by,expires_at) VALUES(?,?,?)")
        .bind(digest(&raw))
        .bind(&session.user.id)
        .bind(expires_at)
        .execute(&s.db)
        .await?;
    Ok(Json(
        json!({"token":raw,"expires_at":expires_at,"url":format!("{}/#invite={raw}",s.config.origin)}),
    ))
}
