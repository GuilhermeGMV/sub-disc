pub mod admin;
pub mod api;
pub mod auth;
pub mod channels;
pub mod config;
pub mod db;
pub mod error;
pub mod media;
pub mod websocket;
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use config::Config;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, broadcast};
use tower_http::services::{ServeDir, ServeFile};
#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);
pub struct Inner {
    pub db: SqlitePool,
    pub config: Config,
    pub events: broadcast::Sender<websocket::Event>,
    pub online: Mutex<HashMap<String, usize>>,
    pub voice: Mutex<HashMap<String, media::Lease>>,
    pub media: media::Hub,
    pub rates: Mutex<HashMap<String, (Instant, u32)>>,
    pub auth_slots: Arc<tokio::sync::Semaphore>,
}
impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}
impl AppState {
    pub fn new(db: SqlitePool, config: Config) -> Self {
        let (events, _) = broadcast::channel(256);
        Self(Arc::new(Inner {
            db,
            config,
            events,
            online: Mutex::new(HashMap::new()),
            voice: Mutex::new(HashMap::new()),
            media: media::Hub::new(),
            rates: Mutex::new(HashMap::new()),
            auth_slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }))
    }
    pub fn emit(&self, event: websocket::Event) {
        let _ = self.events.send(event);
    }
    pub async fn rate(&self, key: String, max: u32) -> error::Result<()> {
        let mut rates = self.rates.lock().await;
        rates.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(60));
        if rates.len() > 10000 {
            return Err(error::AppError(
                StatusCode::TOO_MANY_REQUESTS,
                "Muitas tentativas. Aguarde um minuto.",
            ));
        }
        let entry = rates.entry(key).or_insert((Instant::now(), 0));
        entry.1 += 1;
        if entry.1 > max {
            return Err(error::AppError(
                StatusCode::TOO_MANY_REQUESTS,
                "Muitas tentativas. Aguarde um minuto.",
            ));
        }
        Ok(())
    }
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: String,
}
pub fn app(state: AppState) -> Router {
    let static_dir = state.config.static_dir.clone();
    Router::new()
        .route("/api/health", get(api::health))
        .route("/api/setup", get(auth::setup_status).post(auth::setup))
        .route("/api/login", post(auth::login))
        .route("/api/register", post(auth::register))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/invites", post(auth::invite))
        .route("/api/users", get(api::users).post(admin::create))
        .route(
            "/api/users/{id}",
            axum::routing::patch(admin::update).delete(admin::delete),
        )
        .route("/api/users/{id}/disconnect", post(admin::disconnect))
        .route("/api/state", get(api::snapshot))
        .route("/api/channels", get(channels::list).post(channels::create))
        .route(
            "/api/channels/{id}",
            axum::routing::patch(channels::update).delete(channels::delete),
        )
        .route(
            "/api/channels/{id}/messages",
            get(channels::history).post(channels::send),
        )
        .route("/api/channels/{id}/join", post(media::join))
        .route("/api/media/leave", post(media::leave))
        .route("/api/media/ws", get(media::upgrade))
        .route("/api/media/publish", post(media::publish))
        .route("/api/media/subscribe", post(media::subscribe))
        .route("/api/media/close", post(media::close))
        .route("/api/media/control", post(media::control))
        .route("/api/ws", get(websocket::upgrade))
        .route("/api/{*path}", get(|| async { error::AppError::missing() }))
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(format!("{static_dir}/index.html"))),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state)
}
async fn security(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let requires_origin = (req.method() != Method::GET && req.method() != Method::HEAD)
        || path == "/api/ws"
        || path == "/api/media/ws";
    if requires_origin
        && req.headers().get("origin").and_then(|h| h.to_str().ok())
            != Some(s.config.origin.as_str())
    {
        return axum::response::IntoResponse::into_response(error::AppError::forbidden());
    }
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' blob:; connect-src 'self' ws: wss:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"));
    res
}
