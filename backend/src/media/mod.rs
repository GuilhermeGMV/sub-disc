use crate::{
    AppState,
    auth::Session,
    channels,
    error::{AppError, Result},
    now,
    websocket::Event,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use livekit_api::{
    access_token::{AccessToken, TokenVerifier, VideoGrants},
    services::{
        ServiceError,
        room::{CreateRoomOptions, RoomClient},
    },
    webhooks::WebhookReceiver,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
#[derive(Clone, Serialize)]
pub struct Lease {
    pub channel_id: String,
    pub identity: String,
    pub user_id: String,
    #[serde(skip)]
    pub session_hash: String,
    pub lease: String,
    pub connected: bool,
    pub muted: bool,
    pub camera: bool,
    pub screen: bool,
    #[serde(skip)]
    pub issued_at: i64,
}
fn client(s: &AppState) -> RoomClient {
    RoomClient::with_api_key(
        &s.config.livekit_internal,
        &s.config.livekit_key,
        &s.config.livekit_secret,
    )
}
fn unavailable() -> AppError {
    AppError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Servidor de mídia indisponível. Tente novamente.",
    )
}
fn not_found(e: &ServiceError) -> bool {
    e.to_string().contains("not_found")
}
async fn remove(s: &AppState, lease: &Lease) -> Result<()> {
    match tokio::time::timeout(
        Duration::from_secs(5),
        client(s).remove_participant(&lease.channel_id, &lease.identity),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) if not_found(&e) => Ok(()),
        _ => Err(unavailable()),
    }
}
pub fn mint(s: &AppState, room: &str, identity: &str, name: &str) -> Result<String> {
    AccessToken::with_api_key(&s.config.livekit_key, &s.config.livekit_secret)
        .with_identity(identity)
        .with_name(name)
        .with_ttl(Duration::from_secs(60))
        .with_grants(VideoGrants {
            room_join: true,
            room: room.into(),
            can_publish: Some(true),
            can_subscribe: Some(true),
            can_publish_data: Some(false),
            can_update_own_metadata: Some(false),
            can_publish_sources: vec![
                "microphone".into(),
                "camera".into(),
                "screen_share".into(),
                "screen_share_audio".into(),
            ],
            ..Default::default()
        })
        .to_jwt()
        .map_err(|_| AppError::internal())
}
pub async fn join(
    State(s): State<AppState>,
    user: Session,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    s.rate(format!("join:{}", user.user.id), 15).await?;
    let mut voice = s.voice.lock().await;
    if channels::find(&s, &id).await?.kind != "voice" {
        return Err(AppError::bad("Este canal não é de voz"));
    }
    if let Some(old) = voice.get(&user.user.id) {
        let active = if old.connected {
            let participants = match tokio::time::timeout(
                Duration::from_secs(5),
                client(&s).list_participants(&old.channel_id),
            )
            .await
            {
                Ok(Ok(p)) => p,
                Ok(Err(e)) if not_found(&e) => vec![],
                _ => return Err(unavailable()),
            };
            participants.iter().any(|p| p.identity == old.identity)
        } else {
            now() - old.issued_at < 75
        };
        if active {
            return Err(AppError(
                StatusCode::CONFLICT,
                "Sua conta já está em uma chamada. Saia da chamada na outra aba ou dispositivo.",
            ));
        }
        remove(&s, old).await?;
    }
    // Reclaim orphaned identities after an API restart before issuing a new grant.
    let sf = client(&s);
    let rooms = tokio::time::timeout(Duration::from_secs(5), sf.list_rooms(vec![]))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
    let prefix = format!("{}:", user.user.id);
    for room in rooms {
        let participants =
            tokio::time::timeout(Duration::from_secs(5), sf.list_participants(&room.name))
                .await
                .map_err(|_| unavailable())?
                .map_err(|_| unavailable())?;
        for participant in participants
            .into_iter()
            .filter(|p| p.identity.starts_with(&prefix))
        {
            tokio::time::timeout(
                Duration::from_secs(5),
                sf.remove_participant(&room.name, &participant.identity),
            )
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        }
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        client(&s).create_room(
            &id,
            CreateRoomOptions {
                max_participants: 20,
                empty_timeout: 300,
                ..Default::default()
            },
        ),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    let lease = uuid::Uuid::new_v4().to_string();
    let identity = format!("{}:{lease}", user.user.id);
    let token = mint(&s, &id, &identity, &user.user.username)?;
    voice.insert(
        user.user.id.clone(),
        Lease {
            channel_id: id.clone(),
            identity,
            user_id: user.user.id,
            session_hash: user.hash,
            lease: lease.clone(),
            connected: false,
            muted: true,
            camera: false,
            screen: false,
            issued_at: now(),
        },
    );
    Ok(Json(
        json!({"url":s.config.livekit_url,"token":token,"lease":lease,"room":id}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaveInput {
    lease: String,
}
pub async fn release(s: &AppState, user: &Session, lease: Option<&str>) -> Result<()> {
    let mut voice = s.voice.lock().await;
    if let Some(old) = voice.get(&user.user.id)
        && old.session_hash == user.hash
        && lease.is_none_or(|id| id == old.lease)
    {
        remove(s, old).await?;
        voice.remove(&user.user.id);
        s.emit(Event::PresenceChanged);
    }
    Ok(())
}
/// Revoke the local lease even if LiveKit is temporarily unavailable. A stale
/// token is rejected by the webhook, and every client receives voice_reset.
pub async fn disconnect_user(s: &AppState, id: &str) -> Result<()> {
    let mut voice = s.voice.lock().await;
    let Some(lease) = voice.remove(id) else {
        return Ok(());
    };
    let result = remove(s, &lease).await;
    s.emit(Event::VoiceRemoved { user_id: id.into() });
    s.emit(Event::PresenceChanged);
    result
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
    let mut voice = s.voice.lock().await;
    match tokio::time::timeout(Duration::from_secs(5), client(s).delete_room(id)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) if not_found(&e) => {}
        _ => return Err(unavailable()),
    };
    sqlx::query("DELETE FROM channels WHERE id=?")
        .bind(id)
        .execute(&s.db)
        .await?;
    voice.retain(|_, v| v.channel_id != id);
    s.emit(Event::PresenceChanged);
    Ok(())
}
pub async fn webhook(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<Value>> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let event = WebhookReceiver::new(TokenVerifier::with_api_key(
        &s.config.livekit_key,
        &s.config.livekit_secret,
    ))
    .receive(&body, auth)
    .map_err(|_| AppError::unauthorized())?;
    if let (Some(room), Some(p)) = (event.room, event.participant) {
        let user_id = p.identity.split(':').next().unwrap_or_default();
        let mut voice = s.voice.lock().await;
        match event.event.as_str() {
            "participant_joined" => {
                if let Some(v) = voice
                    .get_mut(user_id)
                    .filter(|v| v.identity == p.identity && v.channel_id == room.name)
                {
                    v.connected = true;
                } else {
                    // Reject stale tokens and calls whose backend lease no longer exists.
                    drop(voice);
                    tokio::time::timeout(
                        Duration::from_secs(5),
                        client(&s).remove_participant(&room.name, &p.identity),
                    )
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(|_| unavailable())?;
                    return Ok(Json(json!({"ok":true})));
                }
            }
            "participant_left" | "participant_connection_aborted"
                if voice
                    .get(user_id)
                    .is_some_and(|v| v.identity == p.identity && v.channel_id == room.name) =>
            {
                voice.remove(user_id);
            }
            _ => {}
        }
        s.emit(Event::PresenceChanged);
    }
    Ok(Json(json!({"ok":true})))
}
