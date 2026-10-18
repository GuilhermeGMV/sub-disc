<script lang="ts">
  import { Track } from 'livekit-client';
  import {
    MicOff,
    Monitor,
    Volume2,
    Maximize2,
    Minimize2,
  } from '@lucide/svelte';
  import { media, participantVolume } from '../stores/media';
  import MediaTrack from './MediaTrack.svelte';
  let expanded = '';
  $: participants = ($media.revision, $media.participants);
  $: videos = participants.flatMap((p) =>
    [...p.videoTrackPublications.values()]
      .filter((pub) => pub.track && !pub.isMuted)
      .map((pub) => ({ p, pub, track: pub.track! })),
  );
</script>

<div class="call-stage">
  {#if !$media.room}<div class="empty">
      <div class="empty-icon"><Volume2 size={36} /></div>
      <h2>Um lugar para estar junto.</h2>
      <p>
        Entre em uma sala de voz na lateral.<br />Microfone, câmera e tela,
        quando você quiser.
      </p>
    </div>
  {:else}
    <div class="call-status">
      <span class="pulse"></span><span data-testid="voice-status"
        >{$media.status === 'connected'
          ? 'Chamada conectada'
          : $media.status === 'reconnecting'
            ? 'Reconectando…'
            : 'Conectando…'}</span
      ><span
        >• {participants.length} pessoa{participants.length === 1
          ? ''
          : 's'}</span
      >
    </div>
    {#if videos.length}<div class:has-expanded={!!expanded} class="video-grid">
        {#each videos as v (v.pub.trackSid)}
          <div
            class:expanded={expanded === v.pub.trackSid}
            class:concealed={!!expanded && expanded !== v.pub.trackSid}
            class="video-tile"
          >
            <MediaTrack track={v.track} participant={v.p} local={v.p.isLocal} />
            <div class="video-label">
              {#if v.track.source === Track.Source.ScreenShare}<Monitor
                  size={14}
                />{/if}{v.p.name} · {v.track.source === Track.Source.ScreenShare
                ? 'Tela'
                : 'Câmera'}<button
                aria-label={expanded ? 'Reduzir vídeo' : 'Ampliar vídeo'}
                onclick={() => (expanded = expanded ? '' : v.pub.trackSid)}
                >{#if expanded}<Minimize2 size={16} />{:else}<Maximize2
                    size={16}
                  />{/if}</button
              >
            </div>
          </div>
        {/each}
      </div>{/if}
    <div class="participant-grid">
      {#each participants as p (p.identity)}
        <div
          class:speaking={p.isSpeaking}
          class="participant-card"
          data-testid="participant"
        >
          <div class="avatar large">
            {(p.name || '?').slice(0, 2).toUpperCase()}
          </div>
          <strong>{p.name}{p.isLocal ? ' (você)' : ''}</strong>
          <span class="participant-state"
            >{#if !p.isMicrophoneEnabled}<MicOff size={14} /> Silenciado{:else if p.isSpeaking}Falando{:else}Na
              chamada{/if}</span
          >
          {#if !p.isLocal}<label class="volume"
              ><Volume2 size={14} /><input
                aria-label={`Volume de ${p.name}`}
                type="range"
                min="0"
                max="1.5"
                step="0.05"
                value="1"
                oninput={(e) =>
                  participantVolume(p.identity, +e.currentTarget.value)}
              /></label
            >{/if}
        </div>
      {/each}
    </div>
    {#if $media.room && !$media.room.canPlaybackAudio}<button
        class="primary"
        onclick={() => {
          $media.room?.startAudio();
        }}>Ativar reprodução de áudio</button
      >{/if}
  {/if}
</div>
