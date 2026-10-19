<script lang="ts">
  import { onMount } from 'svelte';
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
  $: if (expanded && !videos.some((v) => v.pub.trackSid === expanded))
    collapse();
  type FullscreenTile = HTMLElement & {
    webkitRequestFullscreen?: () => Promise<void> | void;
  };
  type FullscreenDocument = Document & {
    webkitFullscreenElement?: Element | null;
    webkitExitFullscreen?: () => Promise<void> | void;
  };
  const fullscreenElement = () =>
    document.fullscreenElement ??
    (document as FullscreenDocument).webkitFullscreenElement ??
    null;
  function collapse() {
    expanded = '';
    const doc = document as FullscreenDocument;
    if (fullscreenElement())
      void Promise.resolve(
        doc.exitFullscreen?.() ?? doc.webkitExitFullscreen?.(),
      ).catch(() => {});
  }
  function expand(sid: string, tile: FullscreenTile | null) {
    expanded = sid;
    // Sem a API (ex.: Safari no iPhone) o CSS de .expanded ocupa a janela.
    if (tile)
      void Promise.resolve(
        tile.requestFullscreen?.() ?? tile.webkitRequestFullscreen?.(),
      ).catch(() => {});
  }
  function fullscreenChanged() {
    if (!fullscreenElement() && expanded) expanded = '';
  }
  onMount(() => {
    const events = ['fullscreenchange', 'webkitfullscreenchange'];
    for (const e of events) document.addEventListener(e, fullscreenChanged);
    return () => {
      for (const e of events)
        document.removeEventListener(e, fullscreenChanged);
    };
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape' && expanded) collapse();
  }}
/>

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
                onclick={(e) =>
                  expanded
                    ? collapse()
                    : expand(
                        v.pub.trackSid,
                        e.currentTarget.closest<HTMLElement>('.video-tile'),
                      )}
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
