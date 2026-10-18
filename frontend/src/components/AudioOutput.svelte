<script lang="ts">
  import { media } from '../stores/media';
  import MediaTrack from './MediaTrack.svelte';
  $: participants =
    ($media.revision, $media.participants.filter((p) => !p.isLocal));
</script>

{#each participants as participant (participant.identity)}
  {#each [...participant.audioTrackPublications.values()].filter((pub) => pub.track) as pub (pub.trackSid)}
    <MediaTrack track={pub.track!} {participant} />
  {/each}
{/each}
