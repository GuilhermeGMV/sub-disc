//! Leases, authenticated signaling and the in-process RTP forwarding service.
mod sfu;
use crate::{
    AppState,
    auth::{self, Session},
    channels,
    error::{AppError, Result},
    now,
    websocket::Event,
};
use axum::{
    Json,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub use sfu::Hub;
use std::time::{Duration, Instant};

#[derive(Clone, Serialize)]
pub struct Lease {
    pub channel_id: String,
    pub identity: String,
    pub user_id: String,
    pub name: String,
    #[serde(skip)]
    pub session_hash: String,
    pub lease: String,
    pub connected: bool,
    pub muted: bool,
    pub camera: bool,
    pub screen: bool,
    pub screen_audio_muted: bool,
    #[serde(skip)]
    pub issued_at: i64,
    #[serde(skip)]
    pub token_hash: String,
    #[serde(skip)]
    pub socket: String,
}

pub async fn join(
    State(s): State<AppState>,
    user: Session,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    s.rate(format!("join:{}", user.user.id), 15).await?;
    if channels::find(&s, &id).await?.kind != "voice" {
        return Err(AppError::bad("Este canal não é de voz"));
    }
    let mut voice = s.voice.lock().await;
    if let Some(old) = voice.get(&user.user.id) {
        if old.connected || now() - old.issued_at < 60 {
            return Err(AppError(
                StatusCode::CONFLICT,
                "Sua conta já está em uma chamada. Saia da chamada na outra aba ou dispositivo.",
            ));
        }
        let old_id = old.lease.clone();
        s.media.remove_lease(&old_id).await;
    }
    if voice.values().filter(|v| v.channel_id == id).count() >= 20 {
        return Err(AppError(
            StatusCode::CONFLICT,
            "A chamada está cheia (20 participantes).",
        ));
    }
    let lease = uuid::Uuid::new_v4().to_string();
    let token = auth::token();
    voice.insert(
        user.user.id.clone(),
        Lease {
            channel_id: id.clone(),
            identity: format!("{}:{lease}", user.user.id),
            user_id: user.user.id,
            name: user.user.username,
            session_hash: user.hash,
            lease: lease.clone(),
            connected: false,
            muted: true,
            camera: false,
            screen: false,
            screen_audio_muted: false,
            issued_at: now(),
            token_hash: auth::digest(&token),
            socket: String::new(),
        },
    );
    Ok(Json(
        json!({"url":"/api/media/ws", "token":token, "lease":lease, "room":id,
        "iceServers":s.config.media_ice_servers}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaveInput {
    pub lease: String,
}
pub async fn release(s: &AppState, user: &Session, lease: Option<&str>) -> Result<()> {
    let removed = {
        let mut voice = s.voice.lock().await;
        if voice
            .get(&user.user.id)
            .is_some_and(|v| v.session_hash == user.hash && lease.is_none_or(|l| l == v.lease))
        {
            voice.remove(&user.user.id)
        } else {
            None
        }
    };
    if let Some(v) = removed {
        s.media.remove_lease(&v.lease).await;
        s.emit(Event::PresenceChanged);
    }
    Ok(())
}
pub async fn leave(
    State(s): State<AppState>,
    user: Session,
    Json(i): Json<LeaveInput>,
) -> Result<Json<Value>> {
    release(&s, &user, Some(&i.lease)).await?;
    Ok(Json(json!({"ok":true})))
}
pub async fn delete_room(s: &AppState, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM channels WHERE id=?")
        .bind(id)
        .execute(&s.db)
        .await?;
    let removed = {
        let mut voice = s.voice.lock().await;
        let ids: Vec<_> = voice
            .values()
            .filter(|v| v.channel_id == id)
            .map(|v| v.lease.clone())
            .collect();
        voice.retain(|_, v| v.channel_id != id);
        ids
    };
    for lease in removed {
        s.media.remove_lease(&lease).await;
    }
    s.emit(Event::PresenceChanged);
    Ok(())
}
async fn authorize(s: &AppState, user: &Session, lease: &str) -> Result<Lease> {
    s.voice
        .lock()
        .await
        .get(&user.user.id)
        .filter(|v| v.lease == lease && v.session_hash == user.hash && v.connected)
        .cloned()
        .ok_or_else(AppError::forbidden)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub lease: String,
    pub sdp: String,
    pub source: Option<sfu::Source>,
    pub publication: Option<String>,
    pub rid: Option<String>,
    pub replace: Option<String>,
}
pub async fn publish(
    State(s): State<AppState>,
    user: Session,
    Json(i): Json<Offer>,
) -> Result<Json<Value>> {
    // Covers a full room (19 peers × 4 sources) plus simulcast layer switches.
    s.rate(format!("media-offer:{}", user.user.id), 240).await?;
    let lease = authorize(&s, &user, &i.lease).await?;
    let source = i.source.ok_or_else(|| AppError::bad("Fonte ausente"))?;
    let result = s.media.publish(&s.config, &lease, source, i.sdp).await?;
    // A lease can be revoked while ICE gathering is in progress.
    if authorize(&s, &user, &i.lease).await.is_err() {
        s.media.remove_lease(&i.lease).await;
        return Err(AppError::forbidden());
    }
    Ok(Json(result))
}
pub async fn subscribe(
    State(s): State<AppState>,
    user: Session,
    Json(i): Json<Offer>,
) -> Result<Json<Value>> {
    s.rate(format!("media-offer:{}", user.user.id), 240).await?;
    let lease = authorize(&s, &user, &i.lease).await?;
    let publication = i
        .publication
        .ok_or_else(|| AppError::bad("Publicação ausente"))?;
    let result = s
        .media
        .subscribe(
            &s.config,
            &lease,
            &publication,
            i.sdp,
            i.rid.as_deref(),
            i.replace.as_deref(),
        )
        .await?;
    if authorize(&s, &user, &i.lease).await.is_err() {
        s.media.remove_lease(&i.lease).await;
        return Err(AppError::forbidden());
    }
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseInput {
    pub lease: String,
    pub id: String,
}
pub async fn close(
    State(s): State<AppState>,
    user: Session,
    Json(i): Json<CloseInput>,
) -> Result<Json<Value>> {
    authorize(&s, &user, &i.lease).await?;
    s.media.remove_owned(&i.id, &i.lease).await;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlInput {
    pub lease: String,
    pub id: String,
    pub paused: bool,
}
pub async fn control(
    State(s): State<AppState>,
    user: Session,
    Json(i): Json<ControlInput>,
) -> Result<Json<Value>> {
    s.rate(format!("media-control:{}", user.user.id), 600)
        .await?;
    authorize(&s, &user, &i.lease).await?;
    s.media.control(&i.id, &i.lease, i.paused).await?;
    Ok(Json(json!({"ok":true})))
}

pub async fn upgrade(
    State(s): State<AppState>,
    user: Session,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    s.rate(format!("media-ws:{}", user.user.id), 60).await?;
    Ok(ws
        .max_message_size(2048)
        .max_frame_size(2048)
        .on_upgrade(move |socket| signal(s, user, socket)))
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Signal {
    Hello {
        lease: String,
        token: String,
    },
    State {
        muted: bool,
        camera: bool,
        screen: bool,
        #[serde(default)]
        screen_audio_muted: bool,
    },
}
async fn signal(s: AppState, user: Session, mut socket: WebSocket) {
    let hello = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
    let Ok(Some(Ok(Message::Text(raw)))) = hello else {
        return;
    };
    let Ok(Signal::Hello { lease, token }) = serde_json::from_str(&raw) else {
        return;
    };
    let epoch = uuid::Uuid::new_v4().to_string();
    {
        let mut voice = s.voice.lock().await;
        let Some(v) = voice.get_mut(&user.user.id).filter(|v| {
            v.lease == lease
                && v.session_hash == user.hash
                && v.token_hash == auth::digest(&token)
                && (v.connected || now() - v.issued_at < 60)
        }) else {
            return;
        };
        v.connected = true;
        v.socket = epoch.clone();
    }
    s.emit(Event::PresenceChanged);
    let (mut sink, mut stream) = socket.split();
    let mut changed = s.media.changed.subscribe();
    let mut events = s.events.subscribe();
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    let mut pong = Instant::now();
    let mut send_snapshot = true;
    loop {
        if send_snapshot {
            let voice = s.voice.lock().await;
            let Some(own) = voice
                .get(&user.user.id)
                .filter(|v| v.lease == lease && v.socket == epoch)
            else {
                break;
            };
            let peers: Vec<_> = voice
                .values()
                .filter(|v| v.channel_id == own.channel_id && v.connected)
                .cloned()
                .collect();
            let publications = s.media.publications(&own.channel_id).await;
            let snapshot = json!({"type":"snapshot", "identity":own.identity, "peers":peers, "publications":publications});
            drop(voice);
            if !matches!(
                tokio::time::timeout(
                    Duration::from_secs(5),
                    sink.send(Message::Text(snapshot.to_string().into()))
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
            send_snapshot = false;
        }
        tokio::select! {
            _ = tick.tick() => {
                let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND expires_at>?)")
                    .bind(&user.hash).bind(now()).fetch_one(&s.db).await.unwrap_or(false);
                if !valid || pong.elapsed() > Duration::from_secs(45) { break }
                send_snapshot = true;
                if sink.send(Message::Ping(vec![].into())).await.is_err() { break }
            }
            event = events.recv() => {
                if matches!(event, Ok(Event::Shutdown)) { break }
                if matches!(event, Ok(Event::SessionRevoked{ref session_hash}) if session_hash == &user.hash) { break }
                send_snapshot = true;
            }
            _ = changed.recv() => send_snapshot = true,
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Pong(_))) => pong = Instant::now(),
                Some(Ok(Message::Text(raw))) => {
                    if s.rate(format!("media-state:{}",user.user.id),120).await.is_err() { break }
                    let Ok(Signal::State{muted,camera,screen,screen_audio_muted}) = serde_json::from_str(&raw) else { break };
                    let mut voice = s.voice.lock().await;
                    let Some(v) = voice.get_mut(&user.user.id).filter(|v| v.lease == lease && v.socket == epoch) else { break };
                    v.muted=muted; v.camera=camera; v.screen=screen; v.screen_audio_muted=screen_audio_muted;
                    s.emit(Event::PresenceChanged);
                }
                Some(Ok(Message::Ping(data))) => { if sink.send(Message::Pong(data)).await.is_err() { break } }
                _ => break,
            }
        }
    }
    let _ = sink.close().await;
    {
        let mut voice = s.voice.lock().await;
        if let Some(v) = voice
            .get_mut(&user.user.id)
            .filter(|v| v.lease == lease && v.socket == epoch)
        {
            v.connected = false;
            v.issued_at = now();
        }
    }
    s.emit(Event::PresenceChanged);
    // The upgrade task must finish immediately; expiry is guarded atomically by epoch.
    let weak = std::sync::Arc::downgrade(&s.0);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let Some(inner) = weak.upgrade() else { return };
        let s = AppState(inner);
        let removed = {
            let mut voice = s.voice.lock().await;
            if voice
                .get(&user.user.id)
                .is_some_and(|v| v.lease == lease && v.socket == epoch && !v.connected)
            {
                voice.remove(&user.user.id)
            } else {
                None
            }
        };
        if let Some(v) = removed {
            s.media.remove_lease(&v.lease).await;
            s.emit(Event::PresenceChanged);
        }
    });
}

/// Reap abandoned joins and stalled transports even when no new request arrives.
pub async fn maintain(s: &AppState) {
    let stale = {
        let mut voice = s.voice.lock().await;
        let stale: Vec<_> = voice
            .values()
            .filter(|v| !v.connected && now() - v.issued_at >= 60)
            .map(|v| (v.user_id.clone(), v.lease.clone()))
            .collect();
        for (user, _) in &stale {
            voice.remove(user);
        }
        stale
    };
    if !stale.is_empty() {
        s.emit(Event::PresenceChanged);
    }
    for (_, lease) in stale {
        s.media.remove_lease(&lease).await;
    }
    s.media.reap().await;
}
