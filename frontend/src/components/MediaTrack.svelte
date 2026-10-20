<script lang="ts">
  import { onMount } from 'svelte';
  import { Track, type Participant } from '../lib/rtc';
  import { applyVolume } from '../stores/media';
  export let track: Track;
  export let participant: Participant;
  export let local = false;
  let container: HTMLDivElement;
  onMount(() => {
    const element = track.attach();
    element.autoplay = true;
    if (element instanceof HTMLVideoElement) {
      element.playsInline = true;
      element.muted = true;
    }
    if (local) element.muted = true;
    container.append(element);
    applyVolume(participant);
    return () => {
      track.detach(element);
      element.remove();
    };
  });
</script>

<div
  bind:this={container}
  class:audio={track.kind === 'audio'}
  class="media-track"
  data-source={track.source}
  data-participant={participant.identity}
></div>
