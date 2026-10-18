export interface User {
  id: string;
  username: string;
  role: 'admin' | 'member';
}
export interface Channel {
  id: string;
  name: string;
  kind: 'text' | 'voice';
  created_at: number;
}
export interface ChatMessage {
  id: number;
  channel_id: string;
  user_id: string;
  username: string;
  content: string;
  created_at: number;
}
export interface VoicePresence {
  user_id: string;
  channel_id: string;
  identity: string;
  lease: string;
  connected: boolean;
  muted: boolean;
  camera: boolean;
  screen: boolean;
}
export interface Snapshot {
  users: User[];
  channels: Channel[];
  online: string[];
  voice: VoicePresence[];
}
export type ServerEvent = { v: 1 } & (
  | { type: 'snapshot'; data: Snapshot }
  | { type: 'message'; message: ChatMessage }
  | { type: 'presence'; online: string[]; voice: VoicePresence[] }
  | { type: 'channels_changed' | 'users_changed' | 'voice_reset' }
);
export function parseEvent(raw: string): ServerEvent | null {
  try {
    const e = JSON.parse(raw);
    if (e.v !== 1) return null;
    if (
      e.type === 'message' &&
      typeof e.message?.id === 'number' &&
      typeof e.message?.content === 'string' &&
      typeof e.message?.channel_id === 'string'
    )
      return e;
    if (
      e.type === 'snapshot' &&
      Array.isArray(e.data?.channels) &&
      Array.isArray(e.data?.users) &&
      Array.isArray(e.data?.online) &&
      Array.isArray(e.data?.voice)
    )
      return e;
    if (
      e.type === 'presence' &&
      Array.isArray(e.online) &&
      Array.isArray(e.voice)
    )
      return e;
    if (
      e.type === 'channels_changed' ||
      e.type === 'users_changed' ||
      e.type === 'voice_reset'
    )
      return e;
  } catch {}
  return null;
}
export function mergeMessages(
  old: ChatMessage[],
  incoming: ChatMessage[],
): ChatMessage[] {
  return [
    ...new Map([...old, ...incoming].map((m) => [m.id, m])).values(),
  ].sort((a, b) => a.id - b.id);
}
