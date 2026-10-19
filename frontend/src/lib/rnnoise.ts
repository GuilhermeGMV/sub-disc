import type {
  AudioProcessorOptions,
  Track,
  TrackProcessor,
} from 'livekit-client';
import {
  RnnoiseWorkletNode,
  loadRnnoise,
} from '@sapphi-red/web-noise-suppressor';
import workletUrl from '@sapphi-red/web-noise-suppressor/rnnoiseWorklet.js?url';
import wasmUrl from '@sapphi-red/web-noise-suppressor/rnnoise.wasm?url';
import simdUrl from '@sapphi-red/web-noise-suppressor/rnnoise_simd.wasm?url';

export const RNNOISE = 'rnnoise';
let wasm: Promise<ArrayBuffer> | undefined;

// Usa um AudioContext próprio a 48 kHz (taxa esperada pelo RNNoise) em vez do
// contexto da sala, que segue a taxa do dispositivo de saída.
export class RnnoiseProcessor implements TrackProcessor<
  Track.Kind.Audio,
  AudioProcessorOptions
> {
  name = RNNOISE;
  processedTrack?: MediaStreamTrack;
  private ctx?: AudioContext;
  private node?: RnnoiseWorkletNode;

  async init({ track }: AudioProcessorOptions) {
    wasm ??= loadRnnoise({ url: wasmUrl, simdUrl }).catch((e) => {
      wasm = undefined;
      throw e;
    });
    const stream = new MediaStream([track]);
    let ctx = new AudioContext({ sampleRate: 48000 });
    try {
      let source: MediaStreamAudioSourceNode;
      try {
        source = ctx.createMediaStreamSource(stream);
      } catch {
        // Firefox não reamostra entre taxas diferentes: usa a do microfone.
        await ctx.close();
        ctx = new AudioContext();
        source = ctx.createMediaStreamSource(stream);
      }
      const [wasmBinary] = await Promise.all([
        wasm,
        ctx.audioWorklet.addModule(workletUrl),
      ]);
      const node = new RnnoiseWorkletNode(ctx, { maxChannels: 1, wasmBinary });
      const destination = ctx.createMediaStreamDestination();
      source.connect(node).connect(destination);
      if (ctx.state === 'suspended') await ctx.resume().catch(() => {});
      this.ctx = ctx;
      this.node = node;
      this.processedTrack = destination.stream.getAudioTracks()[0];
    } catch (e) {
      await ctx.close().catch(() => {});
      throw e;
    }
  }

  async restart(opts: AudioProcessorOptions) {
    await this.destroy();
    await this.init(opts);
  }

  async destroy() {
    this.node?.destroy();
    this.processedTrack?.stop();
    await this.ctx?.close().catch(() => {});
    this.node = this.ctx = this.processedTrack = undefined;
  }
}
