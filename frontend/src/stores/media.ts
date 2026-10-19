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
} from 'livekit-client';
import { post } from '../lib/api';
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
  error: '',
  revision: 0,
};
export const media = writable<MediaState>({ ...initial });
let notify: (data: unknown) => void = () => {};
let lastPresence = '';
let generation = 0;
let videoLevel = 'balanced';
let releaseLock: (() => void) | undefined;
const volumes = new Map<string, number>();
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
      audioCaptureDefaults: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
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
    await s.room.localParticipant.setScreenShareEnabled(
      !s.screen,
      { audio: true, resolution: preset.resolution },
      { screenShareEncoding: preset.encoding },
    );
    refresh();
  }
}
export function participantVolume(identity: string, volume: number) {
  volumes.set(identity, volume);
  const s = get(media);
  s.room?.remoteParticipants.get(identity)?.setVolume(s.deafened ? 0 : volume);
}
export function toggleDeafen() {
  media.update((s) => {
    const deafened = !s.deafened;
    s.room?.remoteParticipants.forEach((p) =>
      p.setVolume(deafened ? 0 : (volumes.get(p.identity) ?? 1)),
    );
    return { ...s, deafened };
  });
}
export function applyVolume(p: Participant) {
  if (p instanceof RemoteParticipant)
    p.setVolume(get(media).deafened ? 0 : (volumes.get(p.identity) ?? 1));
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
