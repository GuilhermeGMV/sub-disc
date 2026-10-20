use crate::{
    AppState,
    auth::Session,
    channels::ChatMessage,
    error::{AppError, Result},
    now,
};
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    #[serde(skip)]
    Shutdown,
    Message {
        message: ChatMessage,
    },
    PresenceChanged,
    ChannelsChanged,
    UsersChanged,
    #[serde(skip)]
    SessionRevoked {
        session_hash: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientEvent {
    Media {
        v: u8,
        lease: String,
        muted: bool,
        camera: bool,
        screen: bool,
    },
}
pub async fn upgrade(
    State(s): State<AppState>,
    session: Session,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    s.rate(format!("ws:{}", session.user.id), 30).await?;
    {
        let mut online = s.online.lock().await;
        let count = online.entry(session.user.id.clone()).or_default();
        if *count >= 8 {
            return Err(AppError::bad("Limite de abas abertas atingido"));
        }
        *count += 1;
    }
    let failed_s = s.clone();
    let failed_id = session.user.id.clone();
    Ok(ws
        .max_message_size(2048)
        .max_frame_size(2048)
        .on_failed_upgrade(move |_| {
            tokio::spawn(async move {
                offline(&failed_s, &failed_id).await;
            });
        })
        .on_upgrade(move |socket| run(s, session, socket)))
}
async fn offline(s: &AppState, id: &str) {
    let mut online = s.online.lock().await;
    if let Some(count) = online.get_mut(id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            online.remove(id);
        }
    }
    s.emit(Event::PresenceChanged);
}
async fn run(s: AppState, session: Session, socket: WebSocket) {
    let mut rx = s.events.subscribe();
    let (mut sink, mut stream) = socket.split();
    s.emit(Event::PresenceChanged);
    if let Ok(snapshot) = crate::api::state(&s).await {
        let _ = sink
            .send(Message::Text(
                serde_json::json!({"v":1,"type":"snapshot","data":snapshot})
                    .to_string()
                    .into(),
            ))
            .await;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
         _=heartbeat.tick()=>{
          if last_pong.elapsed()>Duration::from_secs(60){break}
          let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND expires_at>?)").bind(&session.hash).bind(now()).fetch_one(&s.db).await.unwrap_or(false);
          if !valid{let _=crate::media::release(&s,&session,None).await;break}
          if sink.send(Message::Ping(vec![].into())).await.is_err(){break}
         },
         incoming=stream.next()=>{match incoming {
          Some(Ok(Message::Text(text)))=>{
           if s.rate(format!("ws-event:{}",session.user.id),120).await.is_err(){break}
           match serde_json::from_str::<ClientEvent>(&text){Ok(ClientEvent::Media{v:1,lease,muted,camera,screen})=>{let mut voice=s.voice.lock().await;if let Some(l)=voice.get_mut(&session.user.id).filter(|l|l.lease==lease&&l.session_hash==session.hash){l.muted=muted;l.camera=camera;l.screen=screen;s.emit(Event::PresenceChanged);}else{drop(voice);let _=sink.send(Message::Text(serde_json::json!({"v":1,"type":"voice_reset"}).to_string().into())).await;}},_=>break}
          },Some(Ok(Message::Pong(_)))=>last_pong=Instant::now(),Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>{}
         }},
         event=rx.recv()=>{
          let msg=match event{
           Ok(Event::Shutdown)=>break,
           Ok(Event::SessionRevoked{session_hash})=>{if session_hash==session.hash{break}else{continue}},
           Ok(Event::PresenceChanged)=>serde_json::json!({"v":1,"type":"presence","online":s.online.lock().await.keys().cloned().collect::<Vec<_>>(),"voice":s.voice.lock().await.values().filter(|l|l.connected).cloned().collect::<Vec<_>>()}),
           Ok(e)=>{let mut v=serde_json::to_value(e).unwrap_or_default();v["v"]=1.into();v},
           Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{match crate::api::state(&s).await{Ok(data)=>serde_json::json!({"v":1,"type":"snapshot","data":data}),Err(_)=>break}},
           Err(_)=>break
          };
          if !matches!(tokio::time::timeout(Duration::from_secs(5),sink.send(Message::Text(msg.to_string().into()))).await,Ok(Ok(()))){break}
         }
        }
    }
    let _ = sink.close().await;
    offline(&s, &session.user.id).await;
}
