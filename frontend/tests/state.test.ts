import { describe, it, expect } from 'vitest';
import { parseEvent, mergeMessages, type ChatMessage } from '../src/lib/types';
const message = (id: number): ChatMessage => ({
  id,
  channel_id: 'room',
  content: 'oi',
  created_at: 1,
  user_id: 'user',
  username: 'ana',
});
describe('protocolo e reconexão', () => {
  it('ignora eventos inválidos e versões desconhecidas', () => {
    for (const input of [
      'bad',
      '{}',
      '{"v":2,"type":"users_changed"}',
      '{"v":1,"type":"message"}',
    ])
      expect(parseEvent(input)).toBeNull();
  });
  it('aceita mensagens versionadas', () =>
    expect(
      parseEvent(JSON.stringify({ v: 1, type: 'message', message: message(1) }))
        ?.type,
    ).toBe('message'));
  it('reconcilia o histórico sem duplicatas nem mudança de ordem', () =>
    expect(
      mergeMessages([message(2), message(3)], [message(1), message(2)]).map(
        (m) => m.id,
      ),
    ).toEqual([1, 2, 3]));
});
