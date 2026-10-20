//! Encoded RTP fanout. Each publication and subscription owns one WebRTC transport.
//! No codec or capture device runs in the server.
use super::Lease;
use crate::{
    config::Config,
    error::{AppError, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, broadcast, watch},
    task::AbortHandle,
};
use webrtc::{
    api::{
        APIBuilder,
        interceptor_registry::register_default_interceptors,
        media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8, MediaEngine},
        setting_engine::SettingEngine,
    },
    ice::udp_network::{EphemeralUDP, UDPNetwork},
    ice_transport::{ice_candidate_type::RTCIceCandidateType, ice_server::RTCIceServer},
    interceptor::registry::Registry,
    peer_connection::{
        RTCPeerConnection, configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
    },
    rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication,
    rtp::extension::{HeaderExtension, audio_level_extension::AudioLevelExtension},
    rtp::packet::Packet,
    rtp_transceiver::rtp_codec::{
        RTCRtpCodecCapability, RTCRtpCodecParameters, RTCRtpHeaderExtensionCapability, RTPCodecType,
    },
    track::track_local::{TrackLocal, track_local_static_rtp::TrackLocalStaticRTP},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Microphone,
    Camera,
    ScreenShare,
    ScreenShareAudio,
}
impl Source {
    fn audio(self) -> bool {
        matches!(self, Self::Microphone | Self::ScreenShareAudio)
    }
    fn mime(self) -> &'static str {
        if self.audio() {
            MIME_TYPE_OPUS
        } else {
            MIME_TYPE_VP8
        }
    }
}
#[derive(Clone, Serialize)]
pub struct Publication {
    pub id: String,
    pub identity: String,
    pub source: Source,
    #[serde(skip)]
    room: String,
    #[serde(skip)]
    codec: RTCRtpCodecCapability,
    pub layers: Vec<Layer>,
    pub active_layers: Vec<String>,
    #[serde(skip)]
    ready: bool,
}
#[derive(Clone, Serialize)]
pub struct Layer {
    pub rid: String,
    #[serde(skip)]
    packets: broadcast::Sender<Arc<MediaPacket>>,
    #[serde(skip)]
    ssrc: Arc<AtomicU32>,
    #[serde(skip)]
    keyframe: Arc<Mutex<Instant>>,
}
fn offered_layers(sdp: &str, source: Source) -> Result<Vec<Layer>> {
    let mut rids: Vec<String> = sdp
        .lines()
        .filter_map(|line| {
            let mut fields = line.strip_prefix("a=rid:")?.split_whitespace();
            let rid = fields.next()?;
            (fields.next()? == "send").then(|| rid.to_owned())
        })
        .collect();
    if rids.len() > 3
        || (!rids.is_empty() && source.audio())
        || rids
            .iter()
            .any(|rid| rid.len() > 16 || !rid.bytes().all(|c| c.is_ascii_alphanumeric()))
    {
        return Err(AppError::bad("Camadas de vídeo inválidas"));
    }
    if rids.is_empty() {
        rids.push(String::new());
    }
    let mut unique = rids.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != rids.len() {
        return Err(AppError::bad("Camadas duplicadas"));
    }
    Ok(rids
        .into_iter()
        .map(|rid| Layer {
            rid,
            packets: broadcast::channel(256).0,
            ssrc: Arc::new(AtomicU32::new(0)),
            keyframe: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1))),
        })
        .collect())
}
struct MediaPacket {
    packet: Packet,
    audio_level: Option<AudioLevelExtension>,
}
struct Connection {
    pc: Arc<RTCPeerConnection>,
    lease: String,
    publication: Option<Publication>,
    upstream: Option<String>,
    rid: String,
    enabled: watch::Sender<bool>,
    tasks: Vec<AbortHandle>,
    created: Instant,
}
impl Drop for Connection {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let pc = self.pc.clone();
        tokio::spawn(async move {
            let _ = pc.close().await;
        });
    }
}
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<HashMap<String, Connection>>>,
    pub changed: broadcast::Sender<()>,
}
fn failed(e: impl std::fmt::Display) -> AppError {
    tracing::warn!(error=%e, "WebRTC negotiation failed");
    AppError::bad("Não foi possível negociar a mídia WebRTC.")
}
impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}
impl Hub {
    pub fn new() -> Self {
        let (changed, _) = broadcast::channel(32);
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            changed,
        }
    }
    pub async fn publications(&self, room: &str) -> Vec<Publication> {
        let inner = self.inner.lock().await;
        inner
            .values()
            .filter_map(|c| c.publication.as_ref())
            .filter(|p| p.ready && p.room == room)
            .map(|p| {
                let mut p = p.clone();
                p.active_layers = inner
                    .values()
                    .filter(|c| c.upstream.as_deref() == Some(&p.id) && *c.enabled.borrow())
                    .map(|c| c.rid.clone())
                    .collect();
                p.active_layers.sort();
                p.active_layers.dedup();
                p
            })
            .collect()
    }
    pub async fn control(&self, id: &str, lease: &str, paused: bool) -> Result<()> {
        let inner = self.inner.lock().await;
        let connection = inner
            .get(id)
            .filter(|c| c.lease == lease && c.upstream.is_some())
            .ok_or_else(AppError::forbidden)?;
        if *connection.enabled.borrow() == paused {
            connection.enabled.send_replace(!paused);
            let _ = self.changed.send(());
        }
        Ok(())
    }
    pub async fn remove_owned(&self, id: &str, lease: &str) {
        let mut inner = self.inner.lock().await;
        if inner.get(id).is_some_and(|c| c.lease == lease) {
            remove(&mut inner, id);
        }
        let _ = self.changed.send(());
    }
    pub async fn remove_lease(&self, lease: &str) {
        let mut inner = self.inner.lock().await;
        let ids: Vec<_> = inner
            .iter()
            .filter(|(_, c)| c.lease == lease)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            remove(&mut inner, &id);
        }
        let _ = self.changed.send(());
    }
    pub async fn reap(&self) {
        let mut inner = self.inner.lock().await;
        let stale: Vec<_> = inner
            .iter()
            .filter(|(_, c)| {
                c.created.elapsed() > Duration::from_secs(30)
                    && matches!(
                        c.pc.connection_state(),
                        RTCPeerConnectionState::New
                            | RTCPeerConnectionState::Connecting
                            | RTCPeerConnectionState::Failed
                            | RTCPeerConnectionState::Closed
                    )
            })
            .map(|(id, _)| id.clone())
            .collect();
        if !stale.is_empty() {
            for id in stale {
                remove(&mut inner, &id);
            }
            let _ = self.changed.send(());
        }
    }
    pub async fn shutdown(&self) {
        self.inner.lock().await.clear();
        let _ = self.changed.send(());
    }
    async fn peer(config: &Config) -> Result<Arc<RTCPeerConnection>> {
        let mut media = MediaEngine::default();
        media
            .register_codec(
                RTCRtpCodecParameters {
                    capability: RTCRtpCodecCapability {
                        mime_type: MIME_TYPE_OPUS.into(),
                        clock_rate: 48000,
                        channels: 2,
                        sdp_fmtp_line: "minptime=10;useinbandfec=1;stereo=1;sprop-stereo=1".into(),
                        ..Default::default()
                    },
                    payload_type: 111,
                    ..Default::default()
                },
                RTPCodecType::Audio,
            )
            .map_err(failed)?;
        media.register_default_codecs().map_err(failed)?;
        media
            .register_header_extension(
                RTCRtpHeaderExtensionCapability {
                    uri: "urn:ietf:params:rtp-hdrext:ssrc-audio-level".into(),
                },
                RTPCodecType::Audio,
                None,
            )
            .map_err(failed)?;
        for uri in [
            "urn:ietf:params:rtp-hdrext:sdes:mid",
            "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id",
        ] {
            media
                .register_header_extension(
                    RTCRtpHeaderExtensionCapability { uri: uri.into() },
                    RTPCodecType::Video,
                    None,
                )
                .map_err(failed)?;
        }
        let registry =
            register_default_interceptors(Registry::new(), &mut media).map_err(failed)?;
        let mut settings = SettingEngine::default();
        settings.set_include_loopback_candidate(!config.secure);
        // Link-local IPv6 needs a scope id: every bind fails, scanning the whole
        // UDP range per transport, and its STUN replies are discarded.
        settings.set_ip_filter(Box::new(usable_ip));
        settings.set_udp_network(UDPNetwork::Ephemeral(
            EphemeralUDP::new(config.media_udp_min, config.media_udp_max).map_err(failed)?,
        ));
        if !config.media_public_ip.is_empty() {
            settings.set_nat_1to1_ips(
                vec![config.media_public_ip.clone()],
                RTCIceCandidateType::Host,
            );
        }
        let ice_servers: Vec<RTCIceServer> =
            serde_json::from_value(config.media_ice_servers.clone()).map_err(failed)?;
        let api = APIBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_setting_engine(settings)
            .build();
        Ok(Arc::new(
            api.new_peer_connection(RTCConfiguration {
                ice_servers,
                ..Default::default()
            })
            .await
            .map_err(failed)?,
        ))
    }
    async fn insert(
        &self,
        id: String,
        connection: Connection,
        replace: Option<&str>,
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner
            .values()
            .filter(|c| c.lease == connection.lease)
            .count()
            >= 84
        {
            return Err(AppError::bad("Limite de conexões de mídia"));
        }
        if let Some(p) = &connection.publication {
            if inner.values().any(|c| {
                c.lease == connection.lease
                    && c.publication
                        .as_ref()
                        .is_some_and(|old| old.source == p.source)
            }) {
                return Err(AppError::bad("Esta fonte já foi publicada"));
            }
        } else {
            let duplicates: Vec<_> = inner
                .iter()
                .filter(|(_, c)| c.lease == connection.lease && c.upstream == connection.upstream)
                .collect();
            if !duplicates.is_empty()
                && (duplicates.len() != 1 || replace != Some(duplicates[0].0.as_str()))
            {
                return Err(AppError::bad("Assinatura duplicada"));
            }
            if replace.is_some() && duplicates.is_empty() {
                return Err(AppError::missing());
            }
        }
        inner.insert(id, connection);
        Ok(())
    }
    fn watch(&self, id: String, pc: &Arc<RTCPeerConnection>) {
        let weak = Arc::downgrade(&self.inner);
        let changed = self.changed.clone();
        let peer = Arc::downgrade(pc);
        pc.on_peer_connection_state_change(Box::new(move |state| {
            let (weak, changed, peer, id) =
                (weak.clone(), changed.clone(), peer.clone(), id.clone());
            Box::pin(async move {
                tracing::debug!(transport=%id, ?state, "Media transport state");
                if !matches!(
                    state,
                    RTCPeerConnectionState::Failed
                        | RTCPeerConnectionState::Closed
                        | RTCPeerConnectionState::Disconnected
                ) {
                    return;
                }
                tokio::spawn(async move {
                    if state == RTCPeerConnectionState::Disconnected {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                    }
                    if peer
                        .upgrade()
                        .is_some_and(|p| p.connection_state() == RTCPeerConnectionState::Connected)
                    {
                        return;
                    }
                    if let Some(inner) = weak.upgrade() {
                        remove(&mut *inner.lock().await, &id);
                        let _ = changed.send(());
                    }
                });
            })
        }));
    }
    async fn answer(&self, id: &str, pc: &Arc<RTCPeerConnection>, sdp: String) -> Result<Value> {
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            let offer = RTCSessionDescription::offer(without_link_local(&sdp))?;
            pc.set_remote_description(offer).await?;
            let answer = pc.create_answer(None).await?;
            let mut complete = pc.gathering_complete_promise().await;
            pc.set_local_description(answer).await?;
            complete.recv().await;
            pc.local_description()
                .await
                .ok_or(webrtc::Error::ErrConnectionClosed)
        })
        .await;
        match result {
            Ok(Ok(answer)) => Ok(json!({"id":id,"type":"answer","sdp":answer.sdp})),
            other => {
                remove(&mut *self.inner.lock().await, id);
                Err(failed(format!("{other:?}")))
            }
        }
    }
    pub async fn publish(
        &self,
        config: &Config,
        lease: &Lease,
        source: Source,
        sdp: String,
    ) -> Result<Value> {
        let layers = offered_layers(&sdp, source)?;
        let pc = Self::peer(config).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let publication = Publication {
            id: id.clone(),
            identity: lease.identity.clone(),
            source,
            room: lease.channel_id.clone(),
            codec: RTCRtpCodecCapability {
                mime_type: source.mime().into(),
                ..Default::default()
            },
            layers,
            active_layers: vec![],
            ready: false,
        };
        self.insert(
            id.clone(),
            Connection {
                pc: pc.clone(),
                lease: lease.lease.clone(),
                publication: Some(publication),
                upstream: None,
                rid: String::new(),
                enabled: watch::channel(true).0,
                tasks: vec![],
                created: Instant::now(),
            },
            None,
        )
        .await?;
        tracing::debug!(transport=%id, ?source, "Publish offer");
        self.watch(id.clone(), &pc);
        let weak = Arc::downgrade(&self.inner);
        let changed = self.changed.clone();
        let track_id = id.clone();
        pc.on_track(Box::new(move |track, receiver, _| {
            let (weak, changed, id) = (weak.clone(), changed.clone(), track_id.clone());
            Box::pin(async move {
                if !track
                    .codec()
                    .capability
                    .mime_type
                    .eq_ignore_ascii_case(source.mime())
                {
                    return;
                }
                let audio_level_id = receiver
                    .get_parameters()
                    .await
                    .header_extensions
                    .iter()
                    .find(|e| e.uri == "urn:ietf:params:rtp-hdrext:ssrc-audio-level")
                    .map(|e| e.id as u8);
                let Some(inner) = weak.upgrade() else { return };
                let mut guard = inner.lock().await;
                let Some(connection) = guard.get_mut(&id) else {
                    return;
                };
                let p = connection.publication.as_mut().unwrap();
                let Some(layer) = p.layers.iter().find(|layer| layer.rid == track.rid()) else {
                    return;
                };
                if layer.ssrc.load(Ordering::Relaxed) != 0 {
                    return;
                }
                layer.ssrc.store(track.ssrc(), Ordering::Relaxed);
                tracing::debug!(publication=%id, rid=%track.rid(), ssrc=track.ssrc(), "Layer received");
                let packets = layer.packets.clone();
                p.ready = true;
                p.codec = track.codec().capability;
                let weak = Arc::downgrade(&inner);
                let changed_task = changed.clone();
                let id_task = id.clone();
                let task = tokio::spawn(async move {
                    let mut sequence = Sequence::default();
                    while let Ok((mut packet, _)) = track.read_rtp().await {
                        if !sequence.forward(&mut packet) {
                            continue;
                        }
                        let audio_level = audio_level_id
                            .and_then(|id| packet.header.get_extension(id))
                            .and_then(|v| v.first().copied())
                            .map(|v| AudioLevelExtension {
                                level: v & 0x7f,
                                voice: v & 0x80 != 0,
                            });
                        strip_extensions(&mut packet);
                        let _ = packets.send(Arc::new(MediaPacket {
                            packet,
                            audio_level,
                        }));
                    }
                    if let Some(inner) = weak.upgrade() {
                        remove(&mut *inner.lock().await, &id_task);
                        let _ = changed_task.send(());
                    }
                });
                connection.tasks.push(task.abort_handle());
                drop(guard);
                let _ = changed.send(());
            })
        }));
        self.answer(&id, &pc, sdp).await
    }
    pub async fn subscribe(
        &self,
        config: &Config,
        lease: &Lease,
        publication: &str,
        sdp: String,
        rid: Option<&str>,
        replace: Option<&str>,
    ) -> Result<Value> {
        let (p, upstream) = {
            let inner = self.inner.lock().await;
            let c = inner.get(publication).ok_or_else(AppError::missing)?;
            let p = c
                .publication
                .as_ref()
                .filter(|p| p.ready && p.room == lease.channel_id && p.identity != lease.identity)
                .ok_or_else(AppError::forbidden)?;
            (p.clone(), Arc::downgrade(&c.pc))
        };
        let layer = match rid {
            Some(rid) => p.layers.iter().find(|l| l.rid == rid),
            None => p.layers.last(),
        }
        .ok_or_else(|| AppError::bad("Camada inválida"))?
        .clone();
        let pc = Self::peer(config).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let local = Arc::new(TrackLocalStaticRTP::new(
            p.codec.clone(),
            p.id.clone(),
            p.identity.clone(),
        ));
        let sender = pc
            .add_track(local.clone() as Arc<dyn TrackLocal + Send + Sync>)
            .await
            .map_err(failed)?;
        let (enabled, mut enabled_rx) = watch::channel(true);
        self.insert(
            id.clone(),
            Connection {
                pc: pc.clone(),
                lease: lease.lease.clone(),
                publication: None,
                upstream: Some(publication.into()),
                rid: layer.rid.clone(),
                enabled,
                tasks: vec![],
                created: Instant::now(),
            },
            replace,
        )
        .await?;
        tracing::debug!(transport=%id, %publication, rid=%layer.rid, "Subscribe offer");
        self.watch(id.clone(), &pc);
        let _ = self.changed.send(());
        let mut packets = layer.packets.subscribe();
        let ssrc = layer.ssrc.clone();
        let rtcp_ssrc = ssrc.clone();
        let upstream_rtcp = upstream.clone();
        let keyframe = layer.keyframe.clone();
        let rtcp_keyframe = keyframe.clone();
        let rtcp = tokio::spawn(async move {
            let mut last = Instant::now() - Duration::from_secs(1);
            while let Ok((feedback, _)) = sender.read_rtcp().await {
                if last.elapsed() < Duration::from_millis(200) {
                    continue;
                }
                if feedback.iter().any(|f|f.as_any().is::<PictureLossIndication>() || f.as_any().is::<webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest>()) {
                    pli(&upstream_rtcp,rtcp_ssrc.load(Ordering::Relaxed),&rtcp_keyframe).await; last=Instant::now();
                }
            }
        });
        let video = !p.source.audio();
        let subscriber = Arc::downgrade(&pc);
        let forward = tokio::spawn(async move {
            let mut startup = tokio::time::interval(Duration::from_millis(100));
            let mut waiting_for_keyframe = video;
            loop {
                tokio::select! {
                    changed=enabled_rx.changed()=>{
                        if changed.is_err(){break}
                        if *enabled_rx.borrow(){waiting_for_keyframe=video;}
                    },
                    _=startup.tick(), if waiting_for_keyframe => {
                        let ssrc=ssrc.load(Ordering::Relaxed);
                        if ssrc!=0 && *enabled_rx.borrow() && subscriber.upgrade().is_some_and(|pc|pc.connection_state()==RTCPeerConnectionState::Connected){
                            pli(&upstream,ssrc,&keyframe).await;waiting_for_keyframe=false;
                        }
                    },
                    packet=packets.recv()=>match packet {
                        Ok(packet)=>{
                            if !*enabled_rx.borrow(){continue}
                            let extension=packet.audio_level.map(HeaderExtension::AudioLevel);
                            let extensions=extension.as_slice();
                            // One bad packet must not end the subscription; dropping the
                            // connection aborts this task.
                            if let Err(e)=local.write_rtp_with_extensions(&packet.packet,extensions).await{
                                tracing::debug!(error=%e,"RTP forward failed");
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_))=>{if video {pli(&upstream,ssrc.load(Ordering::Relaxed),&keyframe).await;}},
                        Err(_)=>break,
                    }
                }
            }
        });
        if let Some(c) = self.inner.lock().await.get_mut(&id) {
            c.tasks
                .extend([rtcp.abort_handle(), forward.abort_handle()]);
        } else {
            rtcp.abort();
            forward.abort();
            return Err(AppError::missing());
        }
        self.answer(&id, &pc, sdp).await
    }
}
fn usable_ip(ip: std::net::IpAddr) -> bool {
    !matches!(ip, std::net::IpAddr::V6(v6) if v6.is_unicast_link_local())
}
/// libnice offers link-local IPv6 without a scope id. Replies then never match
/// the request address, and a pair nominated there leaves ICE stuck connecting.
fn without_link_local(sdp: &str) -> String {
    sdp.split_inclusive('\n')
        .filter(|line| {
            !line.starts_with("a=candidate:")
                || line
                    .split_whitespace()
                    .nth(4)
                    .and_then(|address| address.parse().ok())
                    .is_none_or(usable_ip)
        })
        .collect()
}
/// Drops padding-only packets (sender bandwidth probes) and renumbers the rest,
/// so receivers see no gap and do not discard frames waiting for a keyframe.
/// Shared by every subscriber of a layer; tolerates reordering and wrap-around.
#[derive(Default)]
struct Sequence {
    highest: Option<u64>,
    /// Unwrapped numbers of recently dropped packets, ascending.
    dropped: std::collections::VecDeque<u64>,
    /// Dropped packets older than the reorder window.
    settled: u64,
}
impl Sequence {
    const WINDOW: u64 = 4096;
    fn forward(&mut self, packet: &mut Packet) -> bool {
        let number = packet.header.sequence_number;
        let unwrapped = match self.highest {
            None => u64::from(number) + (1 << 16),
            Some(highest) => {
                let candidate = (highest & !0xffff) | u64::from(number);
                if candidate + (1 << 15) < highest {
                    candidate + (1 << 16)
                } else if candidate > highest + (1 << 15) && candidate >= 1 << 16 {
                    candidate - (1 << 16)
                } else {
                    candidate
                }
            }
        };
        let highest = self.highest.map_or(unwrapped, |h| h.max(unwrapped));
        self.highest = Some(highest);
        if unwrapped + Self::WINDOW < highest {
            return false;
        }
        while self
            .dropped
            .front()
            .is_some_and(|d| d + Self::WINDOW < highest)
        {
            self.dropped.pop_front();
            self.settled += 1;
        }
        if packet.payload.is_empty() {
            if let Err(at) = self.dropped.binary_search(&unwrapped) {
                self.dropped.insert(at, unwrapped);
            }
            return false;
        }
        let earlier = self.dropped.partition_point(|d| *d < unwrapped) as u64;
        packet.header.sequence_number = number.wrapping_sub((self.settled + earlier) as u16);
        true
    }
}
/// Audio level is re-mapped by URI; MID, RID and TWCC are transport-local.
fn strip_extensions(packet: &mut Packet) {
    packet.header.extensions.clear();
    // A stale padding count makes rtp 0.14 size and write disagree once the
    // outgoing transport adds its own extension.
    packet.header.extensions_padding = 0;
    packet.header.extension = false;
}
fn remove(inner: &mut HashMap<String, Connection>, id: &str) {
    inner.remove(id);
    inner.retain(|_, c| c.upstream.as_deref() != Some(id));
}
async fn pli(upstream: &Weak<RTCPeerConnection>, ssrc: u32, gate: &Mutex<Instant>) {
    if ssrc == 0 {
        return;
    }
    let mut last = gate.lock().await;
    if last.elapsed() < Duration::from_millis(500) {
        return;
    }
    *last = Instant::now();
    drop(last);
    if let Some(pc) = upstream.upgrade() {
        let _ = pc
            .write_rtcp(&[Box::new(PictureLossIndication {
                sender_ssrc: 0,
                media_ssrc: ssrc,
            })])
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            database_url: String::new(),
            origin: "http://localhost".into(),
            bind: "127.0.0.1:0".into(),
            secure: false,
            setup_key: "test-setup-secret".into(),
            media_ice_servers: json!([]),
            media_public_ip: String::new(),
            media_udp_min: 50000,
            media_udp_max: 51999,
            static_dir: String::new(),
        }
    }
    fn lease() -> Lease {
        Lease {
            channel_id: "room".into(),
            identity: "user:lease".into(),
            user_id: "user".into(),
            name: "User".into(),
            session_hash: "session".into(),
            lease: "lease".into(),
            connected: true,
            muted: false,
            camera: false,
            screen: false,
            screen_audio_muted: false,
            issued_at: crate::now(),
            token_hash: String::new(),
            socket: String::new(),
        }
    }
    #[test]
    fn simulcast_layers_are_bounded_and_single_layer_is_compatible() {
        let layers = offered_layers(
            "a=rid:l send\r\na=rid:m send\r\na=rid:h send\r\n",
            Source::Camera,
        )
        .unwrap();
        assert_eq!(
            layers.iter().map(|l| l.rid.as_str()).collect::<Vec<_>>(),
            vec!["l", "m", "h"]
        );
        assert_eq!(
            offered_layers("v=0", Source::ScreenShare).unwrap()[0].rid,
            ""
        );
        assert!(offered_layers("a=rid:l send\na=rid:l send", Source::Camera).is_err());
        assert!(offered_layers("a=rid:l send", Source::Microphone).is_err());
        assert!(
            offered_layers(
                "a=rid:a send\na=rid:b send\na=rid:c send\na=rid:d send",
                Source::Camera
            )
            .is_err()
        );
    }
    #[test]
    fn padding_probes_are_dropped_without_sequence_gaps() {
        let mut sequence = Sequence::default();
        let mut out = vec![];
        // 65535 is a probe and 2 arrives late; numbering wraps around.
        for (number, probe) in [(65533, false), (65534, false), (65535, true), (0, false)]
            .into_iter()
            .chain([(1, true), (3, false), (2, false), (4, false)])
        {
            let mut packet = Packet::default();
            packet.header.sequence_number = number;
            if !probe {
                packet.payload = vec![1].into();
            }
            if sequence.forward(&mut packet) {
                out.push(packet.header.sequence_number);
            }
        }
        assert_eq!(out, vec![65533, 65534, 65535, 1, 0, 2]);
        // A retransmitted probe is not counted twice.
        let mut probe = Packet::default();
        probe.header.sequence_number = 1;
        assert!(!sequence.forward(&mut probe));
        let mut next = Packet::default();
        next.header.sequence_number = 5;
        next.payload = vec![1].into();
        assert!(sequence.forward(&mut next));
        assert_eq!(next.header.sequence_number, 3);
    }
    #[test]
    fn link_local_ipv6_is_not_gathered() {
        assert!(!usable_ip("fe80::1533:2010:8407:ea00".parse().unwrap()));
        assert!(usable_ip("2804:14d::1".parse().unwrap()));
        assert!(usable_ip("::1".parse().unwrap()));
        assert!(usable_ip("192.168.0.73".parse().unwrap()));
        let sdp = "v=0\r\na=candidate:1 1 UDP 2015363327 fe80::1533:2010:8407:ea00 41000 typ host\r\n\
                   a=candidate:2 1 UDP 2015363071 192.168.0.73 41001 typ host\r\n\
                   a=candidate:3 1 UDP 2015363071 host.local 41002 typ host\r\n";
        assert_eq!(
            without_link_local(sdp),
            "v=0\r\na=candidate:2 1 UDP 2015363071 192.168.0.73 41001 typ host\r\n\
             a=candidate:3 1 UDP 2015363071 host.local 41002 typ host\r\n"
        );
    }
    #[test]
    fn forwarded_packets_drop_padded_extensions() {
        use webrtc::util::{Marshal, Unmarshal};
        // One-byte extension (id 1, one byte) followed by two padding bytes, as Chrome sends.
        let raw: &[u8] = &[
            0x90, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0xbe, 0xde, 0, 1, 0x10, 7, 0, 0, 1, 2, 3,
        ];
        let mut packet = Packet::unmarshal(&mut &raw[..]).unwrap();
        assert!(packet.header.extensions_padding > 0);
        strip_extensions(&mut packet);
        packet.header.set_extension(3, vec![1, 2].into()).unwrap();
        let out = packet.marshal().unwrap();
        let back = Packet::unmarshal(&mut &out[..]).unwrap();
        assert_eq!(&back.payload[..], &[1, 2, 3]);
        assert_eq!(back.header.get_extension(1), None);
        assert_eq!(&back.header.get_extension(3).unwrap()[..], &[1, 2]);
    }
    #[tokio::test]
    async fn rejected_negotiation_releases_source_reservation() {
        let hub = Hub::new();
        let cfg = config();
        for _ in 0..4 {
            assert!(
                hub.publish(&cfg, &lease(), Source::Microphone, "invalid".into())
                    .await
                    .is_err()
            );
            assert!(hub.inner.lock().await.is_empty());
        }
    }
    #[tokio::test]
    async fn source_exclusivity_owner_checks_and_shutdown() {
        let hub = Hub::new();
        let cfg = config();
        let publisher = Hub::peer(&cfg).await.unwrap();
        let track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.into(),
                clock_rate: 48000,
                channels: 2,
                ..Default::default()
            },
            "audio".into(),
            "stream".into(),
        ));
        publisher.add_track(track).await.unwrap();
        let offer = publisher.create_offer(None).await.unwrap();
        let mut complete = publisher.gathering_complete_promise().await;
        publisher.set_local_description(offer).await.unwrap();
        complete.recv().await;
        let answer = hub
            .publish(
                &cfg,
                &lease(),
                Source::Microphone,
                publisher.local_description().await.unwrap().sdp,
            )
            .await
            .unwrap();
        let id = answer["id"].as_str().unwrap();
        assert_eq!(hub.inner.lock().await.len(), 1);
        assert!(
            hub.publish(&cfg, &lease(), Source::Microphone, "invalid".into())
                .await
                .is_err()
        );
        hub.remove_owned(id, "other-lease").await;
        assert_eq!(hub.inner.lock().await.len(), 1);
        hub.remove_owned(id, "lease").await;
        assert!(hub.inner.lock().await.is_empty());
        hub.shutdown().await;
        publisher.close().await.unwrap();
    }
}
