import { describe, expect, it } from 'vitest';
import {
  videoLayer,
  type QualityState,
  type VideoHealth,
} from '../src/lib/adaptive';
const sample = (received: number, lost = 0): VideoHealth => ({
  received,
  lost,
  jitterDelay: received * 0.02,
  jitterCount: received,
  rtt: 0.02,
});
describe('qualidade de vídeo por receptor', () => {
  it('limita a camada pelo tamanho realmente exibido', () => {
    const state: QualityState = { ceiling: 2, healthy: 0 };
    expect(videoLayer(state, sample(0), 240, 3)).toBe(0);
    expect(videoLayer(state, sample(100), 500, 3)).toBe(1);
    expect(videoLayer(state, sample(200), 1000, 3)).toBe(2);
  });
  it('reduz com perda e só recupera após cinco amostras saudáveis', () => {
    const state: QualityState = { ceiling: 2, healthy: 0 };
    videoLayer(state, sample(100), 1000, 3);
    expect(videoLayer(state, sample(180, 20), 1000, 3)).toBe(1);
    for (let i = 1; i < 5; i++)
      expect(videoLayer(state, sample(180 + 100 * i, 20), 1000, 3)).toBe(1);
    expect(videoLayer(state, sample(680, 20), 1000, 3)).toBe(2);
  });
  it('trata contadores reiniciados e mantém uma fonte de camada única válida', () => {
    const state: QualityState = {
      ceiling: 2,
      healthy: 0,
      previous: sample(9000, 100),
    };
    expect(videoLayer(state, sample(0), 1000, 3)).toBe(2);
    expect(videoLayer(state, { ...sample(100), rtt: 0.8 }, 1000, 3)).toBe(1);
    expect(videoLayer(state, sample(200), 1000, 1)).toBe(0);
  });
});
