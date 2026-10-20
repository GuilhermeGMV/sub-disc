/** Receiver-side hysteresis. Each replacement transport starts fresh RTP counters. */
export interface VideoHealth {
  received: number;
  lost: number;
  jitterDelay: number;
  jitterCount: number;
  rtt: number;
}
export interface QualityState {
  ceiling: number;
  healthy: number;
  previous?: VideoHealth;
}
export function videoLayer(
  state: QualityState,
  current: VideoHealth,
  width: number,
  count: number,
) {
  const previous = state.previous;
  state.previous = current;
  if (previous && current.received >= previous.received) {
    const received = current.received - previous.received;
    const lost = Math.max(0, current.lost - previous.lost);
    const samples = current.jitterCount - previous.jitterCount;
    const delay =
      samples > 0 ? (current.jitterDelay - previous.jitterDelay) / samples : 0;
    if (
      (received + lost > 20 && lost / (received + lost) > 0.05) ||
      delay > 0.15 ||
      current.rtt > 0.4
    ) {
      state.ceiling = Math.max(0, state.ceiling - 1);
      state.healthy = 0;
    } else if (received > 20 && ++state.healthy >= 5) {
      state.ceiling = Math.min(count - 1, state.ceiling + 1);
      state.healthy = 0;
    }
  }
  const display = width <= 320 ? 0 : width <= 640 ? 1 : count - 1;
  return Math.max(0, Math.min(count - 1, state.ceiling, display));
}
