import { writable, get } from 'svelte/store';
import {
  Room,
  RoomEvent,
  ConnectionState,
  Track,
  Participant,
  RemoteParticipant,
  VideoPresets,
  ScreenSharePresets,
  AudioPresets,
  type LocalAudioTrack,
} from 'livekit-client';
import { post } from '../lib/api';
import { RNNOISE, RnnoiseProcessor } from '../lib/rnnoise';
export interface MediaState {
  room: Room | null;
  channelId: string | null;
  lease: string | null;
  status: string;
  participants: Participant[];
  muted: boolean;
  deafened: boolean;
  camera: boolean;
  screen: boolean;
  screenAudio: 'none' | 'on' | 'off';
  error: string;
  revision: number;
}
const initial: MediaState = {
  room: null,
  channelId: null,
  lease: null,
  status: 'disconnected',
  participants: [],
  muted: true,
  deafened: false,
  camera: false,
  screen: false,
  screenAudio: 'none',
  error: '',
  revision: 0,
};
export const media = writable<MediaState>({ ...initial });
let notify: (data: unknown) => void = () => {};
let lastPresence = '';
let generation = 0;
let videoLevel = 'balanced';
let releaseLock: (() => void) | undefined;
const VOLUMES_KEY = 'subdisc:volumes';
// Identidade LiveKit é `usuario:lease`; o volume fica salvo por usuário.
const userOf = (identity: string) => identity.split(':')[0];
const volumes = loadVolumes();
function loadVolumes() {
  try {
    const saved = JSON.parse(localStorage.getItem(VOLUMES_KEY) ?? '{}');
    return new Map(
      Object.entries(saved).filter(
        (e): e is [string, number] =>
          typeof e[1] === 'number' && e[1] >= 0 && e[1] <= 1.5,
      ),
    );
  } catch {
    return new Map<string, number>();
  }
}
export type NoiseMode = 'off' | 'browser' | 'strong';
export interface AudioSettings {
  noise: NoiseMode;
  echo: boolean;
  agc: boolean;
  shareAudio: boolean;
}
const AUDIO_KEY = 'subdisc:audio';
export const audioSettings = writable<AudioSettings>(loadAudioSettings());
function loadAudioSettings(): AudioSettings {
  const fallback: AudioSettings = {
    noise: 'browser',
    echo: true,
    agc: true,
    shareAudio: true,
  };
  try {
    const saved = JSON.parse(localStorage.getItem(AUDIO_KEY) ?? '{}');
    return {
      noise: ['off', 'browser', 'strong'].includes(saved.noise)
        ? saved.noise
        : fallback.noise,
      echo: typeof saved.echo === 'boolean' ? saved.echo : fallback.echo,
      agc: typeof saved.agc === 'boolean' ? saved.agc : fallback.agc,
      shareAudio:
        typeof saved.shareAudio === 'boolean'
          ? saved.shareAudio
          : fallback.shareAudio,
    };
  } catch {
    return fallback;
  }
}
const captureOptions = (a: AudioSettings) => ({
  echoCancellation: a.echo,
  noiseSuppression: a.noise === 'browser',
  autoGainControl: a.agc,
});
const micTrack = (room: Room) =>
  room.localParticipant.getTrackPublication(Track.Source.Microphone)
    ?.audioTrack as LocalAudioTrack | undefined;
// Sem deviceId o LiveKit reabre o microfone ignorando as demais restrições.
async function restartMic(track: LocalAudioTrack, a: AudioSettings) {
  const deviceId = (await track.getDeviceId(false)) ?? 'default';
  await track.restartTrack({ ...captureOptions(a), deviceId });
}
let noiseQueue = Promise.resolve();
function syncNoiseProcessor(room: Room) {
  noiseQueue = noiseQueue.then(async () => {
    const track = micTrack(room);
    if (!track) return;
    const strong = get(audioSettings).noise === 'strong';
    const active = track.getProcessor()?.name === RNNOISE;
    try {
      if (strong && !active) await track.setProcessor(new RnnoiseProcessor());
      else if (!strong && active) await track.stopProcessor();
    } catch {
      if (!strong) return;
      await restartMic(track, {
        ...get(audioSettings),
        noise: 'browser',
      }).catch(() => {});
      mediaError(
        new Error(
          'Supressão forte indisponível neste navegador; usando a do navegador.',
        ),
      );
    }
  });
  return noiseQueue;
}
export async function setAudioSettings(next: AudioSettings) {
  const previous = get(audioSettings);
  audioSettings.set(next);
  try {
    localStorage.setItem(AUDIO_KEY, JSON.stringify(next));
  } catch {
    // Sem armazenamento: vale só nesta sessão.
  }
  const room = get(media).room;
  const micChanged =
    previous.noise !== next.noise ||
    previous.echo !== next.echo ||
    previous.agc !== next.agc;
  if (!room || !micChanged) return;
  room.options.audioCaptureDefaults = {
    ...room.options.audioCaptureDefaults,
    ...captureOptions(next),
  };
  const track = micTrack(room);
  if (track) await restartMic(track, next);
  await syncNoiseProcessor(room);
}
export type VolumeSource = 'mic' | 'screen';
const volumeKey = (identity: string, source: VolumeSource) =>
  source === 'screen' ? `${userOf(identity)}:screen` : userOf(identity);
export function savedVolume(identity: string, source: VolumeSource = 'mic') {
  return volumes.get(volumeKey(identity, source)) ?? 1;
}
// setVolume sem fonte só afeta o microfone; o som da tela é ajustado à parte.
function applyVolumes(p: RemoteParticipant, deafened: boolean) {
  p.setVolume(deafened ? 0 : savedVolume(p.identity), Track.Source.Microphone);
  p.setVolume(
    deafened ? 0 : savedVolume(p.identity, 'screen'),
    Track.Source.ScreenShareAudio,
  );
}
const screenAudioState = (pub?: { isMuted: boolean }) =>
  !pub ? 'none' : pub.isMuted ? 'off' : 'on';
export function setPresenceSender(sender: (data: unknown) => void) {
  notify = sender;
  lastPresence = '';
  sync();
}
function sync() {
  const s = get(media);
  if (!s.room || !s.lease) return;
  const data = {
    v: 1,
    type: 'media',
    lease: s.lease,
    muted: !s.room.localParticipant.isMicrophoneEnabled,
    camera: s.room.localParticipant.isCameraEnabled,
    screen: s.room.localParticipant.isScreenShareEnabled,
  };
  const serialized = JSON.stringify(data);
  if (serialized !== lastPresence) {
    lastPresence = serialized;
    notify(data);
  }
}
function refresh() {
  media.update((s) => {
    if (!s.room) return s;
    const p = s.room.localParticipant;
    return {
      ...s,
      participants: [p, ...s.room.remoteParticipants.values()],
      muted: !p.isMicrophoneEnabled,
      camera: p.isCameraEnabled,
      screen: p.isScreenShareEnabled,
      screenAudio: screenAudioState(
        p.getTrackPublication(Track.Source.ScreenShareAudio),
      ),
      revision: s.revision + 1,
    };
  });
  sync();
}
export function mediaError(e: unknown) {
  media.update((s) => ({
    ...s,
    error: e instanceof Error ? e.message : String(e),
  }));
}
export async function joinVoice(channelId: string) {
  if (get(media).status !== 'disconnected') return;
  const attempt = ++generation;
  media.update((s) => ({ ...s, status: 'connecting', error: '' }));
  try {
    if (navigator.locks) {
      const acquired = await new Promise<boolean>((resolve) => {
        void navigator.locks.request(
          'subdisc-voice',
          { ifAvailable: true },
          async (lock) => {
            if (!lock) {
              resolve(false);
              return;
            }
            await new Promise<void>((done) => {
              if (attempt !== generation) {
                done();
                resolve(false);
                return;
              }
              releaseLock = done;
              resolve(true);
            });
          },
        );
      });
      if (!acquired)
        throw new Error('Você já está em uma chamada em outra aba.');
    }
    const access = await post<{ url: string; token: string; lease: string }>(
      `/channels/${channelId}/join`,
    );
    if (attempt !== generation) {
      await post('/media/leave', { lease: access.lease });
      return;
    }
    const room = new Room({
      adaptiveStream: true,
      dynacast: true,
      audioCaptureDefaults: captureOptions(get(audioSettings)),
      videoCaptureDefaults: { resolution: VideoPresets.h720.resolution },
      publishDefaults: {
        audioPreset: AudioPresets.musicHighQuality,
        videoCodec: 'vp8',
        simulcast: true,
        videoEncoding: VideoPresets.h720.encoding,
        screenShareEncoding: ScreenSharePresets.h1080fps15.encoding,
      },
    });
    media.update((s) => ({ ...s, room, channelId, lease: access.lease }));
    for (const event of [
      RoomEvent.ParticipantConnected,
      RoomEvent.ParticipantDisconnected,
      RoomEvent.TrackSubscribed,
      RoomEvent.TrackUnsubscribed,
      RoomEvent.TrackMuted,
      RoomEvent.TrackUnmuted,
      RoomEvent.LocalTrackPublished,
      RoomEvent.LocalTrackUnpublished,
      RoomEvent.ActiveSpeakersChanged,
      RoomEvent.TrackPublished,
      RoomEvent.TrackUnpublished,
    ])
      room.on(event, refresh);
    room.on(RoomEvent.ConnectionStateChanged, (status) => {
      if (get(media).room === room) media.update((s) => ({ ...s, status }));
    });
    room.on(RoomEvent.MediaDevicesError, mediaError);
    room.on(RoomEvent.LocalTrackPublished, (pub) => {
      if (pub.source === Track.Source.Microphone) void syncNoiseProcessor(room);
    });
    room.on(RoomEvent.MediaDevicesChanged, () => {
      const track = room.localParticipant.getTrackPublication(
        Track.Source.Microphone,
      )?.audioTrack;
      if (track?.mediaStreamTrack.readyState === 'ended')
        mediaError(
          new Error(
            'Microfone desconectado. Selecione outro dispositivo e ative o microfone novamente.',
          ),
        );
      refresh();
    });
    room.on(RoomEvent.AudioPlaybackStatusChanged, refresh);
    room.on(RoomEvent.Disconnected, () => {
      if (get(media).room === room) {
        const s = get(media);
        media.set({
          ...initial,
          error: 'Chamada encerrada. Você pode entrar novamente.',
        });
        releaseLock?.();
        releaseLock = undefined;
        if (s.lease)
          void post('/media/leave', { lease: s.lease }).catch(mediaError);
      }
    });
    await room.connect(access.url, access.token);
    if (attempt !== generation) {
      await room.disconnect();
      return;
    }
    refresh();
    try {
      await room.localParticipant.setMicrophoneEnabled(true);
    } catch {
      mediaError(
        new Error(
          'Não foi possível abrir o microfone. Verifique a permissão e o dispositivo.',
        ),
      );
    }
    refresh();
  } catch (e) {
    if (attempt === generation) {
      await leaveVoice().catch(mediaError);
      mediaError(e);
    }
  }
}
export async function leaveVoice() {
  ++generation;
  const s = get(media);
  const unlock = releaseLock;
  releaseLock = undefined;
  media.set({ ...initial, status: 'disconnecting' });
  try {
    await s.room?.disconnect();
    if (s.lease) await post('/media/leave', { lease: s.lease });
  } finally {
    unlock?.();
    media.set({ ...initial });
  }
}
export async function toggleMic() {
  const s = get(media);
  if (s.room?.state === ConnectionState.Connected) {
    await s.room.localParticipant.setMicrophoneEnabled(s.muted);
    refresh();
  }
}
export async function toggleCamera() {
  const s = get(media);
  if (s.room) {
    const preset =
      videoLevel === 'low'
        ? VideoPresets.h360
        : videoLevel === 'detail'
          ? VideoPresets.h1080
          : VideoPresets.h720;
    await s.room.localParticipant.setCameraEnabled(
      !s.camera,
      { resolution: preset.resolution },
      { videoEncoding: preset.encoding },
    );
    refresh();
  }
}
export async function toggleScreen(quality = 'balanced') {
  const s = get(media);
  if (s.room) {
    const preset =
      quality === 'detail'
        ? ScreenSharePresets.h1080fps30
        : quality === 'low'
          ? ScreenSharePresets.h720fps5
          : ScreenSharePresets.h1080fps15;
    const sharing = !s.screen;
    const { shareAudio } = get(audioSettings);
    const local = s.room.localParticipant;
    await local.setScreenShareEnabled(
      sharing,
      {
        // Som da tela vai sem os filtros de voz, em estéreo.
        audio: shareAudio && {
          echoCancellation: false,
          noiseSuppression: false,
          autoGainControl: false,
        },
        systemAudio: 'include',
        suppressLocalAudioPlayback: false,
        resolution: preset.resolution,
      },
      {
        screenShareEncoding: preset.encoding,
        audioPreset: AudioPresets.musicHighQualityStereo,
        forceStereo: true,
        dtx: false,
      },
    );
    refresh();
    if (
      sharing &&
      shareAudio &&
      local.isScreenShareEnabled &&
      !local.getTrackPublication(Track.Source.ScreenShareAudio)
    )
      mediaError(
        new Error(
          'A tela está sendo compartilhada sem som. Para enviar o áudio, marque "Compartilhar áudio" ao escolher o que compartilhar. No Linux e no macOS o Chrome só envia o som de abas; Firefox e Safari não enviam som de tela.',
        ),
      );
  }
}
export async function toggleScreenAudio() {
  const pub = get(media).room?.localParticipant.getTrackPublication(
    Track.Source.ScreenShareAudio,
  );
  if (!pub) return;
  await (pub.isMuted ? pub.unmute() : pub.mute());
  refresh();
}
export function participantVolume(
  identity: string,
  volume: number,
  source: VolumeSource = 'mic',
) {
  // Sincronize o controle na tela e no cartão do participante.
  volumes.set(
    volumeKey(identity, source),
    Number.isFinite(volume) ? Math.max(0, Math.min(1.5, volume)) : 1,
  );
  try {
    localStorage.setItem(
      VOLUMES_KEY,
      JSON.stringify(Object.fromEntries(volumes)),
    );
  } catch {
    // Sem armazenamento (ex.: janela privada): vale só nesta sessão.
  }
  const s = get(media);
  const p = s.room?.remoteParticipants.get(identity);
  if (p) applyVolumes(p, s.deafened);
  media.update((current) => ({ ...current, revision: current.revision + 1 }));
}
export function toggleDeafen() {
  media.update((s) => {
    const deafened = !s.deafened;
    s.room?.remoteParticipants.forEach((p) => applyVolumes(p, deafened));
    return { ...s, deafened };
  });
}
export function applyVolume(p: Participant) {
  if (p instanceof RemoteParticipant) applyVolumes(p, get(media).deafened);
}
export async function devices() {
  return Room.getLocalDevices(undefined, false);
}
export async function switchDevice(
  kind: 'audioinput' | 'audiooutput' | 'videoinput',
  id: string,
) {
  await get(media).room?.switchActiveDevice(kind, id);
}
export async function quality(level: string) {
  videoLevel = level;
  const room = get(media).room;
  if (!room) return;
  const preset =
    level === 'low'
      ? VideoPresets.h360
      : level === 'detail'
        ? VideoPresets.h1080
        : VideoPresets.h720;
  const track = room.localParticipant.getTrackPublication(
    Track.Source.Camera,
  )?.videoTrack;
  if (track) await track.restartTrack({ resolution: preset.resolution });
}
