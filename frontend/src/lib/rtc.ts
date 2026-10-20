/** Subdisc WebRTC client. SDP over authenticated HTTP; room snapshots over WS. */
import { api } from './api';
import { videoLayer, type QualityState, type VideoHealth } from './adaptive';
const mediaPost = <T>(path: string, data: unknown, timeout = 18000) =>
  api<T>(path, {
    method: 'POST',
    body: JSON.stringify(data),
    signal: AbortSignal.timeout(timeout),
  });
export enum ConnectionState {
  Connected = 'connected',
  Disconnected = 'disconnected',
  Connecting = 'connecting',
  Reconnecting = 'reconnecting',
}
export enum RoomEvent {
  ParticipantConnected,
  ParticipantDisconnected,
  TrackSubscribed,
  TrackUnsubscribed,
  TrackMuted,
  TrackUnmuted,
  LocalTrackPublished,
  LocalTrackUnpublished,
  ActiveSpeakersChanged,
  TrackPublished,
  TrackUnpublished,
  ConnectionStateChanged,
  MediaDevicesError,
  MediaDevicesChanged,
  AudioPlaybackStatusChanged,
  Disconnected,
}
export interface AudioProcessorOptions {
  track: MediaStreamTrack;
}
export interface TrackProcessor<
  K = Track.Kind.Audio,
  O = AudioProcessorOptions,
> {
  name: string;
  processedTrack?: MediaStreamTrack;
  init(options: O): Promise<void>;
  restart(options: O): Promise<void>;
  destroy(): Promise<void>;
}
type Resolution = { width: number; height: number; frameRate: number };
type Encoding = { maxBitrate: number; maxFramerate?: number };
const preset = (
  width: number,
  height: number,
  frameRate: number,
  maxBitrate: number,
) => ({
  resolution: { width, height, frameRate },
  encoding: { maxBitrate, maxFramerate: frameRate },
});
export const VideoPresets = {
  h360: preset(640, 360, 20, 450000),
  h720: preset(1280, 720, 30, 1700000),
  h1080: preset(1920, 1080, 30, 3000000),
};
export const ScreenSharePresets = {
  h720fps5: preset(1280, 720, 5, 800000),
  h1080fps15: preset(1920, 1080, 15, 2500000),
  h1080fps30: preset(1920, 1080, 30, 5000000),
};
export const AudioPresets = {
  musicHighQuality: { maxBitrate: 96000 },
  musicHighQualityStereo: { maxBitrate: 128000 },
};
type CaptureOptions = MediaTrackConstraints & { resolution?: Resolution };
interface PublishOptions {
  audioPreset?: Encoding;
  videoEncoding?: Encoding;
  screenShareEncoding?: Encoding;
  forceStereo?: boolean;
  dtx?: boolean;
}
interface RoomOptions {
  audioCaptureDefaults?: MediaTrackConstraints;
  videoCaptureDefaults?: CaptureOptions;
  publishDefaults?: PublishOptions & { videoCodec?: string };
}
interface Access {
  lease: string;
  iceServers: RTCIceServer[];
}
interface Peer {
  identity: string;
  name: string;
  muted: boolean;
  camera: boolean;
  screen: boolean;
  screen_audio_muted?: boolean;
}
interface PublicationInfo {
  id: string;
  identity: string;
  source: Track.Source;
  layers: { rid: string }[];
  active_layers: string[];
}
interface Snapshot {
  type: 'snapshot';
  identity: string;
  peers: Peer[];
  publications: PublicationInfo[];
}
interface Transport {
  pc: RTCPeerConnection;
  id?: string;
  publication?: string;
  track?: Track;
  busy?: boolean;
  retryAt?: number;
  failedSince?: number;
  rid?: string;
  paused?: boolean;
  quality?: QualityState;
  switchedAt?: number;
  switchFailures?: number;
}

export class Track {
  readonly kind: Track.Kind;
  private elements = new Set<HTMLMediaElement>();
  private gain?: GainNode;
  private audioSource?: MediaElementAudioSourceNode;
  private volume = 1;
  private senderUpdates = Promise.resolve();
  private activeSignature = '';
  private activeSender?: RTCRtpSender;
  processor?: TrackProcessor;
  sender?: RTCRtpSender;
  encoding?: Encoding;
  receiver?: RTCRtpReceiver;
  constructor(
    public mediaStreamTrack: MediaStreamTrack,
    public source: Track.Source,
    readonly room: Room,
    public sid = '',
  ) {
    this.kind = mediaStreamTrack.kind as Track.Kind;
  }
  attach(): HTMLMediaElement {
    const element = document.createElement(
      this.kind === Track.Kind.Audio ? 'audio' : 'video',
    );
    const stream = new MediaStream([this.mediaStreamTrack]);
    element.srcObject = stream;
    if (this.kind === Track.Kind.Audio) {
      const ctx = this.room.audioContext();
      this.audioSource = ctx.createMediaElementSource(element);
      this.gain = ctx.createGain();
      this.gain.gain.value = this.volume;
      this.audioSource.connect(this.gain).connect(ctx.destination);
    }
    element.srcObject = stream;
    element.autoplay = true;
    if (this.room.output && this.kind === Track.Kind.Audio)
      void this.output(this.room.output).catch((e) => this.room.error(e));
    this.elements.add(element);
    void element.play().catch(() => {
      this.room.canPlaybackAudio = false;
      this.room.emit(RoomEvent.AudioPlaybackStatusChanged);
    });
    return element;
  }
  detach(element: HTMLMediaElement) {
    element.pause();
    element.srcObject = null;
    this.elements.delete(element);
    if (!this.elements.size) {
      this.audioSource?.disconnect();
      this.gain?.disconnect();
    }
  }
  setVolume(volume: number) {
    this.volume = volume;
    if (this.gain) this.gain.gain.value = volume;
  }
  async output(id: string) {
    const ctx = this.room.audioContext() as AudioContext & {
      setSinkId?: (id: string) => Promise<void>;
    };
    if (ctx.setSinkId) await ctx.setSinkId(id);
    else if (id)
      throw new Error('Seleção de saída indisponível neste navegador');
  }
  async play() {
    for (const e of this.elements) await e.play();
  }
  async getDeviceId(_exact = false) {
    return this.mediaStreamTrack.getSettings().deviceId;
  }
  getProcessor() {
    return this.processor;
  }
  async setProcessor(processor: TrackProcessor) {
    await this.stopProcessor();
    await processor.init({ track: this.mediaStreamTrack });
    if (processor.processedTrack)
      await this.sender?.replaceTrack(processor.processedTrack);
    this.processor = processor;
    this.room.resetMeter(this);
  }
  async stopProcessor() {
    if (this.processor) {
      await this.sender?.replaceTrack(this.mediaStreamTrack);
      await this.processor.destroy();
      this.processor = undefined;
      this.room.resetMeter(this);
    }
  }
  private updateSender(update: (params: RTCRtpSendParameters) => void) {
    this.senderUpdates = this.senderUpdates
      .catch(() => {})
      .then(async () => {
        const sender = this.sender;
        if (!sender || sender.getParameters().encodings.length === 0) return;
        const params = sender.getParameters();
        update(params);
        await sender.setParameters(params);
      });
    return this.senderUpdates;
  }
  async setEncoding(encoding: Encoding) {
    this.encoding = { ...encoding };
    // Audio and single-layer publishers retain their original limits.
    await this.updateSender((params) => {
      for (const item of params.encodings) {
        const scale = item.rid === 'l' ? 0.12 : item.rid === 'm' ? 0.4 : 1;
        item.maxBitrate = Math.round(encoding.maxBitrate * scale);
        if (encoding.maxFramerate)
          item.maxFramerate =
            item.rid === 'l'
              ? Math.min(15, encoding.maxFramerate)
              : encoding.maxFramerate;
      }
    });
  }
  async activeLayers(rids: string[]) {
    const signature = [...rids].sort().join(',');
    if (this.activeSender === this.sender && this.activeSignature === signature)
      return;
    const sender = this.sender;
    if (!sender || sender.transport?.state !== 'connected') return;
    await this.updateSender((params) => {
      for (const encoding of params.encodings)
        encoding.active = rids.includes(encoding.rid ?? '');
    });
    this.activeSender = sender;
    this.activeSignature = signature;
  }
  videoDemand() {
    if (document.visibilityState === 'hidden') return 0;
    let width = 0;
    for (const element of this.elements) {
      const rect = element.getBoundingClientRect();
      if (
        rect.width > 0 &&
        rect.height > 0 &&
        rect.bottom > 0 &&
        rect.top < innerHeight &&
        rect.right > 0 &&
        rect.left < innerWidth &&
        getComputedStyle(element).visibility !== 'hidden'
      )
        width = Math.max(width, rect.width * Math.min(devicePixelRatio, 2));
    }
    return width;
  }
  replaceRemote(native: MediaStreamTrack, receiver: RTCRtpReceiver) {
    const old = this.mediaStreamTrack;
    native.enabled = old.enabled;
    this.mediaStreamTrack = native;
    this.receiver = receiver;
    for (const element of this.elements) {
      element.srcObject = new MediaStream([native]);
      void element.play().catch(() => {});
    }
    old.stop();
  }
  async restartTrack(options: CaptureOptions) {
    const { resolution, ...rest } = options;
    const current = this.mediaStreamTrack.getSettings().deviceId;
    const constraints = {
      ...this.mediaStreamTrack.getConstraints(),
      ...rest,
      ...resolution,
      deviceId: rest.deviceId ?? (current ? { exact: current } : undefined),
    };
    const stream = await navigator.mediaDevices.getUserMedia(
      this.kind === Track.Kind.Audio
        ? { audio: constraints }
        : { video: constraints },
    );
    const replacement = stream.getTracks()[0];
    replacement.enabled = this.mediaStreamTrack.enabled;
    const old = this.mediaStreamTrack;
    if (this.processor) {
      await this.processor.restart({ track: replacement });
      await this.sender?.replaceTrack(
        this.processor.processedTrack ?? replacement,
      );
    } else await this.sender?.replaceTrack(replacement);
    this.mediaStreamTrack = replacement;
    this.room.resetMeter(this);
    old.stop();
    for (const e of this.elements) e.srcObject = new MediaStream([replacement]);
  }
  stop() {
    for (const e of [...this.elements]) this.detach(e);
    this.mediaStreamTrack.stop();
    void this.processor?.destroy();
  }
}
export namespace Track {
  export enum Source {
    Microphone = 'microphone',
    Camera = 'camera',
    ScreenShare = 'screen_share',
    ScreenShareAudio = 'screen_share_audio',
  }
  export enum Kind {
    Audio = 'audio',
    Video = 'video',
  }
}
export class LocalAudioTrack extends Track {}
export class Publication {
  constructor(
    public trackSid: string,
    public source: Track.Source,
    public track: Track,
    public owner: Participant,
  ) {}
  get isMuted() {
    return !this.track.mediaStreamTrack.enabled;
  }
  get audioTrack() {
    return this.track.kind === Track.Kind.Audio ? this.track : undefined;
  }
  get videoTrack() {
    return this.track.kind === Track.Kind.Video ? this.track : undefined;
  }
  async mute() {
    this.track.mediaStreamTrack.enabled = false;
    this.owner.room.stateChanged();
  }
  async unmute() {
    this.track.mediaStreamTrack.enabled = true;
    this.owner.room.stateChanged();
  }
}
export class Participant {
  identity = '';
  name = '';
  isSpeaking = false;
  readonly trackPublications = new Map<string, Publication>();
  constructor(
    readonly room: Room,
    readonly isLocal = false,
  ) {}
  get audioTrackPublications() {
    return new Map([...this.trackPublications].filter(([, p]) => p.audioTrack));
  }
  get videoTrackPublications() {
    return new Map([...this.trackPublications].filter(([, p]) => p.videoTrack));
  }
  getTrackPublication(source: Track.Source) {
    return [...this.trackPublications.values()].find(
      (p) => p.source === source,
    );
  }
  get isMicrophoneEnabled() {
    const p = this.getTrackPublication(Track.Source.Microphone);
    return !!p && !p.isMuted;
  }
  get isCameraEnabled() {
    const p = this.getTrackPublication(Track.Source.Camera);
    return !!p && !p.isMuted;
  }
  get isScreenShareEnabled() {
    const p = this.getTrackPublication(Track.Source.ScreenShare);
    return !!p && !p.isMuted;
  }
}
export class RemoteParticipant extends Participant {
  setVolume(volume: number, source: Track.Source) {
    this.getTrackPublication(source)?.track.setVolume(volume);
  }
}
export class LocalParticipant extends Participant {
  constructor(room: Room) {
    super(room, true);
  }
  async setMicrophoneEnabled(enabled: boolean) {
    let p = this.getTrackPublication(Track.Source.Microphone);
    if (!p && enabled) {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: this.room.options.audioCaptureDefaults ?? true,
      });
      p = await this.room.publish(
        stream.getAudioTracks()[0],
        Track.Source.Microphone,
      );
    }
    if (p) await (enabled ? p.unmute() : p.mute());
  }
  async setCameraEnabled(
    enabled: boolean,
    capture?: CaptureOptions,
    options?: PublishOptions,
  ) {
    const p = this.getTrackPublication(Track.Source.Camera);
    if (!enabled) {
      if (p) await this.room.unpublish(p);
      return;
    }
    if (p) return;
    const { resolution, ...constraints } = {
      ...this.room.options.videoCaptureDefaults,
      ...capture,
    };
    const stream = await navigator.mediaDevices.getUserMedia({
      video: { ...constraints, ...resolution },
    });
    await this.room.publish(
      stream.getVideoTracks()[0],
      Track.Source.Camera,
      options,
    );
  }
  async setScreenShareEnabled(
    enabled: boolean,
    capture?: {
      audio?: boolean | MediaTrackConstraints;
      systemAudio?: string;
      suppressLocalAudioPlayback?: boolean;
      resolution?: Resolution;
    },
    options?: PublishOptions,
  ) {
    if (!enabled) {
      for (const source of [
        Track.Source.ScreenShareAudio,
        Track.Source.ScreenShare,
      ]) {
        const p = this.getTrackPublication(source);
        if (p) await this.room.unpublish(p);
      }
      return;
    }
    if (this.isScreenShareEnabled) return;
    const stream = await navigator.mediaDevices.getDisplayMedia({
      video: capture?.resolution ?? true,
      audio: capture?.audio
        ? ({
            ...(typeof capture.audio === 'object' ? capture.audio : {}),
            suppressLocalAudioPlayback:
              capture.suppressLocalAudioPlayback ?? false,
          } as MediaTrackConstraints)
        : false,
      systemAudio: 'include',
    } as DisplayMediaStreamOptions);
    try {
      const video = stream.getVideoTracks()[0];
      await this.room.publish(video, Track.Source.ScreenShare, options);
      video.onended = () => {
        void this.setScreenShareEnabled(false).catch((e) => this.room.error(e));
      };
      const audio = stream.getAudioTracks()[0];
      if (audio)
        await this.room.publish(audio, Track.Source.ScreenShareAudio, options);
    } catch (e) {
      stream.getTracks().forEach((t) => t.stop());
      await this.setScreenShareEnabled(false);
      throw e;
    }
  }
}

export class Room {
  state = ConnectionState.Disconnected;
  readonly localParticipant = new LocalParticipant(this);
  readonly remoteParticipants = new Map<string, RemoteParticipant>();
  canPlaybackAudio = true;
  output = '';
  private listeners = new Map<RoomEvent, ((...args: any[]) => void)[]>();
  private transports = new Set<Transport>();
  private access?: Access;
  private ws?: WebSocket;
  private token = '';
  private url = '';
  private ended = false;
  private ctx?: AudioContext;
  private snapshot?: Snapshot;
  private pending = new Set<string>();
  private timer?: ReturnType<typeof setInterval>;
  private reconnecting?: Promise<void>;
  private lastSignal = 0;
  private measuring = false;
  private adapting = false;
  private meters = new Map<Track, { energy: number; duration: number }>();
  constructor(public options: RoomOptions = {}) {}
  on(event: RoomEvent, callback: (...args: any[]) => void) {
    this.listeners.set(event, [...(this.listeners.get(event) ?? []), callback]);
    return this;
  }
  emit(event: RoomEvent, ...args: any[]) {
    for (const cb of this.listeners.get(event) ?? []) cb(...args);
  }
  error(e: unknown) {
    this.emit(RoomEvent.MediaDevicesError, e);
  }
  private status(state: ConnectionState) {
    this.state = state;
    this.emit(RoomEvent.ConnectionStateChanged, state);
  }
  audioContext() {
    if (!this.ctx) {
      this.ctx = new AudioContext();
      this.ctx.onstatechange = () => {
        this.canPlaybackAudio = this.ctx?.state === 'running';
        this.emit(RoomEvent.AudioPlaybackStatusChanged);
      };
    }
    if (this.ctx.state === 'suspended')
      void this.ctx.resume().catch(() => {
        this.canPlaybackAudio = false;
        this.emit(RoomEvent.AudioPlaybackStatusChanged);
      });
    return this.ctx;
  }
  async connect(url: string, token: string, access: Access) {
    this.url = url;
    this.token = token;
    this.access = access;
    this.ended = false;
    this.status(ConnectionState.Connecting);
    await this.openSignal();
    let tick = 0;
    this.timer = setInterval(() => {
      this.watchSignal();
      if (tick++ % 5 === 0) {
        void this.reconcile();
        void this.adapt();
      }
      void this.measure();
    }, 200);
    navigator.mediaDevices.addEventListener('devicechange', this.deviceChange);
  }
  private deviceChange = () => this.emit(RoomEvent.MediaDevicesChanged);
  private openSignal(): Promise<void> {
    return new Promise((resolve, reject) => {
      const target = new URL(this.url, location.href);
      target.protocol = target.protocol === 'https:' ? 'wss:' : 'ws:';
      const ws = new WebSocket(target);
      this.ws = ws;
      let ready = false;
      const timeout = setTimeout(() => {
        reject(new Error('Tempo de conexão de mídia esgotado'));
        ws.close();
      }, 12000);
      ws.onopen = () =>
        ws.send(
          JSON.stringify({
            type: 'hello',
            lease: this.access!.lease,
            token: this.token,
          }),
        );
      ws.onmessage = (event) => {
        if (this.ws !== ws) return;
        this.lastSignal = Date.now();
        let snapshot: Snapshot;
        try {
          snapshot = JSON.parse(event.data);
          if (
            snapshot.type !== 'snapshot' ||
            !Array.isArray(snapshot.peers) ||
            !Array.isArray(snapshot.publications)
          )
            throw Error('Sinalização inválida');
        } catch (e) {
          this.error(e);
          return;
        }
        this.snapshot = snapshot;
        this.localParticipant.identity = snapshot.identity;
        this.localParticipant.name =
          snapshot.peers.find((p) => p.identity === snapshot.identity)?.name ??
          '';
        if (!ready) {
          ready = true;
          clearTimeout(timeout);
          this.status(ConnectionState.Connected);
          resolve();
        }
        void this.reconcile();
      };
      ws.onclose = () => {
        clearTimeout(timeout);
        if (!ready) reject(new Error('Conexão de mídia recusada'));
        else if (!this.ended && this.ws === ws) void this.reconnect();
      };
      ws.onerror = () => {
        if (!ready) reject(new Error('Sinalização indisponível'));
      };
    });
  }
  /** Snapshots arrive at least every 15 s; a closing or silent socket is dead. */
  private watchSignal() {
    const ws = this.ws;
    if (
      !ws ||
      this.ended ||
      this.reconnecting ||
      this.state !== ConnectionState.Connected ||
      (ws.readyState === WebSocket.OPEN && Date.now() - this.lastSignal < 40000)
    )
      return;
    ws.onclose = ws.onmessage = null;
    ws.close();
    void this.reconnect();
  }
  private reconnect(): Promise<void> {
    if (this.reconnecting) return this.reconnecting;
    this.reconnecting = (async () => {
      this.status(ConnectionState.Reconnecting);
      const started = Date.now();
      for (
        let attempt = 0;
        !this.ended && Date.now() - started < 45000;
        attempt++
      ) {
        await new Promise((r) =>
          setTimeout(r, Math.min(500 * 2 ** attempt, 5000)),
        );
        if (this.ended) return;
        try {
          await this.openSignal();
          this.stateChanged();
          return;
        } catch {
          /* bounded retry with original authenticated lease */
        }
      }
      if (!this.ended) await this.disconnect();
    })().finally(() => {
      this.reconnecting = undefined;
    });
    return this.reconnecting;
  }
  private async negotiate(transport: Transport, body: Record<string, unknown>) {
    const pc = transport.pc;
    await pc.setLocalDescription(await pc.createOffer());
    await new Promise<void>((resolve, reject) => {
      if (pc.iceGatheringState === 'complete') {
        resolve();
        return;
      }
      const timeout = setTimeout(() => {
        pc.removeEventListener('icegatheringstatechange', changed);
        reject(Error('ICE gathering timeout'));
      }, 10000);
      const changed = () => {
        if (pc.iceGatheringState === 'complete') {
          clearTimeout(timeout);
          pc.removeEventListener('icegatheringstatechange', changed);
          resolve();
        }
      };
      pc.addEventListener('icegatheringstatechange', changed);
    });
    const answer = await mediaPost<{
      id: string;
      sdp: string;
      type: RTCSdpType;
    }>(`/media/${body.publication ? 'subscribe' : 'publish'}`, {
      ...body,
      lease: this.access!.lease,
      sdp: pc.localDescription!.sdp,
    });
    transport.id = answer.id;
    if (this.ended || !this.transports.has(transport)) {
      await this.dispose(transport);
      throw Error('Chamada encerrada');
    }
    await pc.setRemoteDescription({ type: 'answer', sdp: answer.sdp });
  }
  private peer(): Transport {
    const transport: Transport = {
      pc: new RTCPeerConnection({ iceServers: this.access?.iceServers ?? [] }),
    };
    this.transports.add(transport);
    return transport;
  }
  private addSender(transport: Transport, track: Track) {
    const native = track.processor?.processedTrack ?? track.mediaStreamTrack;
    const transceiver = transport.pc.addTransceiver(native, {
      direction: 'sendonly',
      streams: [new MediaStream([native])],
      ...(native.kind === 'video'
        ? {
            sendEncodings: [
              { rid: 'l', scaleResolutionDownBy: 4 },
              { rid: 'm', scaleResolutionDownBy: 2 },
              { rid: 'h', scaleResolutionDownBy: 1 },
            ],
          }
        : {}),
    });
    const codecs = RTCRtpSender.getCapabilities(native.kind)?.codecs.filter(
      (codec) =>
        codec.mimeType.toLowerCase() ===
        (native.kind === 'audio' ? 'audio/opus' : 'video/vp8'),
    );
    if (codecs?.length) transceiver.setCodecPreferences(codecs);
    track.sender = transceiver.sender;
  }
  async publish(
    native: MediaStreamTrack,
    source: Track.Source,
    options?: PublishOptions,
  ): Promise<Publication> {
    const track =
      native.kind === 'audio'
        ? new LocalAudioTrack(native, source, this)
        : new Track(native, source, this);
    const transport = this.peer();
    transport.track = track;
    this.addSender(transport, track);
    try {
      await this.negotiate(transport, { source });
      track.sid = transport.id!;
      const encoding =
        source === Track.Source.Camera
          ? (options?.videoEncoding ??
            this.options.publishDefaults?.videoEncoding)
          : source === Track.Source.ScreenShare
            ? (options?.screenShareEncoding ??
              this.options.publishDefaults?.screenShareEncoding)
            : (options?.audioPreset ?? AudioPresets.musicHighQuality);
      if (encoding) await track.setEncoding(encoding);
      const publication = new Publication(
        track.sid,
        source,
        track,
        this.localParticipant,
      );
      this.localParticipant.trackPublications.set(track.sid, publication);
      native.addEventListener(
        'ended',
        () => {
          if (this.localParticipant.trackPublications.has(track.sid))
            void this.unpublish(publication).catch((e) => this.error(e));
        },
        { once: true },
      );
      this.emit(RoomEvent.LocalTrackPublished, publication);
      this.stateChanged();
      return publication;
    } catch (e) {
      track.stop();
      await this.dispose(transport);
      throw e;
    }
  }
  async unpublish(p: Publication) {
    this.localParticipant.trackPublications.delete(p.trackSid);
    p.track.stop();
    this.unmeter(p.track);
    for (const t of this.transports)
      if (t.track === p.track) await this.dispose(t);
    this.emit(RoomEvent.LocalTrackUnpublished, p);
    this.stateChanged();
  }
  stateChanged() {
    const p = this.localParticipant;
    if (this.ws?.readyState === WebSocket.OPEN)
      this.ws.send(
        JSON.stringify({
          type: 'state',
          muted: !p.isMicrophoneEnabled,
          camera: p.isCameraEnabled,
          screen: p.isScreenShareEnabled,
          screen_audio_muted:
            p.getTrackPublication(Track.Source.ScreenShareAudio)?.isMuted ??
            false,
        }),
      );
    this.emit(RoomEvent.TrackMuted);
  }
  private async reconcile() {
    const snapshot = this.snapshot;
    if (!snapshot || this.ended || this.state !== ConnectionState.Connected)
      return;
    for (const info of snapshot.publications) {
      const local = this.localParticipant.trackPublications.get(info.id)?.track;
      if (local?.kind === Track.Kind.Video)
        void local.activeLayers(info.active_layers).catch((e) => this.error(e));
    }
    for (const [id, p] of this.remoteParticipants)
      if (!snapshot.peers.some((v) => v.identity === id)) {
        for (const pub of p.trackPublications.values()) this.detach(p, pub);
        this.remoteParticipants.delete(id);
        this.emit(RoomEvent.ParticipantDisconnected, p);
      }
    for (const peer of snapshot.peers)
      if (peer.identity !== snapshot.identity) {
        let p = this.remoteParticipants.get(peer.identity);
        if (!p) {
          p = new RemoteParticipant(this);
          p.identity = peer.identity;
          p.name = peer.name;
          this.remoteParticipants.set(p.identity, p);
          this.emit(RoomEvent.ParticipantConnected, p);
        }
        for (const pub of p.trackPublications.values()) {
          const info = snapshot.publications.find((v) => v.id === pub.trackSid);
          if (!info) {
            this.detach(p, pub);
            continue;
          }
          const enabled =
            pub.source === Track.Source.Microphone
              ? !peer.muted
              : pub.source === Track.Source.Camera
                ? peer.camera
                : pub.source === Track.Source.ScreenShareAudio
                  ? !peer.screen_audio_muted
                  : peer.screen;
          pub.track.mediaStreamTrack.enabled = enabled;
        }
      }
    for (const t of [...this.transports]) {
      if (
        t.busy ||
        (t.retryAt ?? 0) > Date.now() ||
        !['failed', 'closed'].includes(t.pc.connectionState)
      )
        continue;
      t.busy = true;
      if (t.publication) {
        const p = [...this.remoteParticipants.values()].find((p) =>
          p.trackPublications.has(t.publication!),
        );
        if (p) this.detach(p, p.trackPublications.get(t.publication)!);
        else await this.dispose(t);
      } else if (t.track) {
        // Rebuild a failed transport without reopening a device or the screen picker.
        const track = t.track;
        const pub = this.localParticipant.trackPublications.get(track.sid);
        t.failedSince ??= Date.now();
        let replacement: Transport | undefined;
        try {
          await this.dispose(t);
          replacement = this.peer();
          replacement.track = track;
          this.addSender(replacement, track);
          await this.negotiate(replacement, { source: track.source });
          if (
            this.ended ||
            !pub ||
            this.localParticipant.trackPublications.get(track.sid) !== pub
          ) {
            await this.dispose(replacement);
            continue;
          }
          if (track.encoding) await track.setEncoding(track.encoding);
          this.localParticipant.trackPublications.delete(track.sid);
          track.sid = replacement.id!;
          if (pub) {
            pub.trackSid = track.sid;
            this.localParticipant.trackPublications.set(track.sid, pub);
          }
          this.stateChanged();
        } catch (e) {
          if (replacement) await this.dispose(replacement);
          if (
            !this.ended &&
            pub &&
            this.localParticipant.trackPublications.get(track.sid) === pub &&
            track.mediaStreamTrack.readyState === 'live' &&
            Date.now() - t.failedSince < 45000
          ) {
            // Keep the authorized device open through temporary negotiation failures.
            t.busy = false;
            t.retryAt = Date.now() + 1500;
            this.transports.add(t);
          } else if (!this.ended) {
            this.error(e);
            if (pub) await this.unpublish(pub);
          }
        }
      }
    }
    for (const info of snapshot.publications) {
      const p = this.remoteParticipants.get(info.identity);
      if (!p || p.trackPublications.has(info.id) || this.pending.has(info.id))
        continue;
      this.pending.add(info.id);
      void this.subscribe(p, info)
        .catch((e) => {
          if (!this.ended) this.error(e);
        })
        .finally(() => this.pending.delete(info.id));
    }
    this.emit(RoomEvent.TrackMuted);
  }
  private async subscribe(
    p: RemoteParticipant,
    info: PublicationInfo,
    rid = info.layers[0]?.rid ?? '',
    replace?: Transport,
  ) {
    const t = this.peer();
    t.publication = info.id;
    t.rid = rid;
    t.busy = true;
    t.quality = replace?.quality ?? {
      ceiling: Math.max(0, info.layers.length - 1),
      healthy: 0,
    };
    t.quality.previous = undefined;
    t.switchedAt = replace ? Date.now() : 0;
    t.pc.addTransceiver(
      info.source === Track.Source.Microphone ||
        info.source === Track.Source.ScreenShareAudio
        ? 'audio'
        : 'video',
      { direction: 'recvonly' },
    );
    let received: RTCTrackEvent | undefined;
    let stage: HTMLVideoElement | undefined;
    t.pc.ontrack = (event) => {
      if (this.ended || !this.transports.has(t)) return;
      received = event;
      if (replace) return;
      const track = new Track(event.track, info.source, this, info.id);
      t.track = track;
      track.receiver = event.receiver;
      p.trackPublications.set(
        info.id,
        new Publication(info.id, info.source, track, p),
      );
      this.emit(RoomEvent.TrackSubscribed, track, p);
    };
    try {
      await this.negotiate(t, {
        publication: info.id,
        rid,
        replace: replace?.id,
      });
      if (replace) {
        if (!received) throw Error('Faixa de vídeo ausente');
        stage = document.createElement('video');
        stage.muted = true;
        stage.playsInline = true;
        stage.srcObject = new MediaStream([received.track]);
        let deadline: ReturnType<typeof setTimeout> | undefined;
        try {
          await Promise.race([
            stage.play(),
            new Promise((_, reject) => {
              deadline = setTimeout(
                () => reject(Error('Tempo de troca de camada esgotado')),
                10000,
              );
            }),
          ]);
        } finally {
          clearTimeout(deadline);
        }
        if (
          this.ended ||
          !this.transports.has(replace) ||
          p.trackPublications.get(info.id)?.track !== replace.track
        ) {
          await this.dispose(t);
          return;
        }
        const track = replace.track!;
        track.replaceRemote(received.track, received.receiver);
        t.track = track;
        await this.dispose(replace);
      }
      t.busy = false;
    } catch (e) {
      await this.dispose(t);
      if (!replace && p.trackPublications.get(info.id)?.track === t.track) {
        p.trackPublications.delete(info.id);
        t.track?.stop();
        this.emit(RoomEvent.TrackUnsubscribed, t.track, p);
      }
      throw e;
    } finally {
      if (stage) {
        stage.pause();
        stage.srcObject = null;
      }
    }
  }
  private async adapt() {
    if (this.adapting || this.ended || this.state !== ConnectionState.Connected)
      return;
    this.adapting = true;
    try {
      let change:
        | {
            participant: RemoteParticipant;
            info: PublicationInfo;
            transport: Transport;
            rid: string;
          }
        | undefined;
      for (const t of this.transports) {
        if (
          !t.publication ||
          !t.id ||
          t.busy ||
          t.track?.kind !== Track.Kind.Video ||
          t.pc.connectionState !== 'connected'
        )
          continue;
        const info = this.snapshot?.publications.find(
          (p) => p.id === t.publication,
        );
        if (!info) continue;
        const width = t.track.videoDemand();
        const paused = width === 0;
        if (paused !== !!t.paused) {
          await mediaPost(
            '/media/control',
            { lease: this.access!.lease, id: t.id, paused },
            5000,
          );
          t.paused = paused;
        }
        if (paused || info.layers.length < 2) continue;
        const health: VideoHealth = {
          received: 0,
          lost: 0,
          jitterDelay: 0,
          jitterCount: 0,
          rtt: 0,
        };
        (await t.pc.getStats()).forEach((stat) => {
          if (stat.type === 'inbound-rtp' && stat.kind === 'video') {
            health.received += stat.packetsReceived ?? 0;
            health.lost += stat.packetsLost ?? 0;
            health.jitterDelay += stat.jitterBufferDelay ?? 0;
            health.jitterCount += stat.jitterBufferEmittedCount ?? 0;
          } else if (
            stat.type === 'candidate-pair' &&
            stat.state === 'succeeded'
          )
            health.rtt = Math.max(health.rtt, stat.currentRoundTripTime ?? 0);
        });
        const quality = t.quality!;
        const rid =
          info.layers[videoLayer(quality, health, width, info.layers.length)]
            .rid;
        if (
          !change &&
          rid !== t.rid &&
          Date.now() - (t.switchedAt ?? 0) >
            4000 * 2 ** Math.min(t.switchFailures ?? 0, 4)
        ) {
          const participant = this.remoteParticipants.get(info.identity);
          if (participant) change = { participant, info, transport: t, rid };
        }
      }
      if (change) {
        const { participant, info, transport, rid } = change;
        transport.busy = true;
        try {
          await this.subscribe(participant, info, rid, transport);
        } catch (e) {
          // The current layer keeps playing; a sender may have suspended the
          // requested one under CPU or bandwidth limits.
          transport.switchFailures = (transport.switchFailures ?? 0) + 1;
          throw e;
        } finally {
          transport.busy = false;
          transport.switchedAt = Date.now();
        }
      }
    } catch (e) {
      // Adaptation is best effort and not actionable by the user.
      if (!this.ended) console.warn('Adaptação de vídeo', e);
    } finally {
      this.adapting = false;
    }
  }
  private detach(p: Participant, pub: Publication) {
    p.trackPublications.delete(pub.trackSid);
    pub.track.stop();
    this.unmeter(pub.track);
    for (const t of this.transports)
      if (t.publication === pub.trackSid) void this.dispose(t);
    this.emit(RoomEvent.TrackUnsubscribed, pub.track, p);
  }
  private async dispose(t: Transport) {
    this.transports.delete(t);
    t.pc.close();
    if (t.id && this.access)
      await mediaPost(
        '/media/close',
        { lease: this.access.lease, id: t.id },
        2000,
      ).catch(() => {});
  }
  resetMeter(track: Track) {
    this.unmeter(track);
  }
  private unmeter(track: Track) {
    this.meters.delete(track);
  }
  private async measure() {
    if (this.measuring || this.ended) return;
    this.measuring = true;
    try {
      await Promise.all(
        [this.localParticipant, ...this.remoteParticipants.values()].map(
          async (p) => {
            const track = p.getTrackPublication(Track.Source.Microphone)?.track;
            if (!track) return;
            const endpoint = p.isLocal ? track.sender : track.receiver;
            if (!endpoint) return;
            const stats = await endpoint.getStats().catch(() => undefined);
            if (
              !stats ||
              this.ended ||
              p.getTrackPublication(Track.Source.Microphone)?.track !== track
            )
              return;
            let level = 0;
            let energy = 0;
            let duration = 0;
            stats.forEach((report) => {
              if (
                (report.type === 'media-source' ||
                  report.type === 'inbound-rtp') &&
                (report.kind === 'audio' || report.mediaType === 'audio')
              ) {
                level = Math.max(level, report.audioLevel ?? 0);
                energy += report.totalAudioEnergy ?? 0;
                duration += report.totalSamplesDuration ?? 0;
              }
            });
            const previous = this.meters.get(track);
            this.meters.set(track, { energy, duration });
            const meanEnergy =
              previous && duration > previous.duration
                ? (energy - previous.energy) / (duration - previous.duration)
                : 0;
            const speaking =
              p.isMicrophoneEnabled && (level > 0.005 || meanEnergy > 0.000025);
            if (speaking !== p.isSpeaking) {
              p.isSpeaking = speaking;
              this.emit(RoomEvent.ActiveSpeakersChanged);
            }
          },
        ),
      );
    } finally {
      this.measuring = false;
    }
  }
  async startAudio() {
    await this.ctx?.resume();
    for (const p of this.remoteParticipants.values())
      for (const pub of p.audioTrackPublications.values())
        await pub.track.play();
    this.canPlaybackAudio = true;
    this.emit(RoomEvent.AudioPlaybackStatusChanged);
  }
  static async getLocalDevices(_kind?: unknown, _request = false) {
    return navigator.mediaDevices.enumerateDevices();
  }
  async switchActiveDevice(
    kind: 'audioinput' | 'audiooutput' | 'videoinput',
    id: string,
  ) {
    if (kind === 'audiooutput') {
      this.output = id;
      for (const p of this.remoteParticipants.values())
        for (const pub of p.audioTrackPublications.values())
          await pub.track.output(id);
      return;
    }
    const p = this.localParticipant.getTrackPublication(
      kind === 'audioinput' ? Track.Source.Microphone : Track.Source.Camera,
    );
    if (kind === 'audioinput')
      this.options.audioCaptureDefaults = {
        ...this.options.audioCaptureDefaults,
        deviceId: { exact: id },
      };
    else
      this.options.videoCaptureDefaults = {
        ...this.options.videoCaptureDefaults,
        deviceId: { exact: id },
      };
    if (p)
      await p.track.restartTrack({
        ...(kind === 'audioinput' ? this.options.audioCaptureDefaults : {}),
        deviceId: { exact: id },
      });
  }
  async disconnect() {
    if (this.ended) return;
    this.ended = true;
    clearInterval(this.timer);
    navigator.mediaDevices.removeEventListener(
      'devicechange',
      this.deviceChange,
    );
    this.ws?.close();
    for (const p of [
      this.localParticipant,
      ...this.remoteParticipants.values(),
    ])
      for (const pub of p.trackPublications.values()) {
        pub.track.stop();
        this.unmeter(pub.track);
      }
    await Promise.all([...this.transports].map((t) => this.dispose(t)));
    this.localParticipant.trackPublications.clear();
    this.remoteParticipants.clear();
    await this.ctx?.close();
    this.ctx = undefined;
    this.status(ConnectionState.Disconnected);
    this.emit(RoomEvent.Disconnected);
  }
}
