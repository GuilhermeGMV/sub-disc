<script lang="ts">
  import { onMount } from 'svelte';
  import { X } from '@lucide/svelte';
  import {
    devices,
    switchDevice,
    quality,
    mediaError,
    audioSettings,
    setAudioSettings,
    type AudioSettings,
    type NoiseMode,
  } from '../stores/media';
  export let close: () => void;
  export let level = 'balanced';
  let list: MediaDeviceInfo[] = [];
  let error = '';
  async function reload() {
    try {
      list = await devices();
    } catch {
      error = 'Não foi possível listar os dispositivos.';
    }
  }
  const updateAudio = (patch: Partial<AudioSettings>) =>
    setAudioSettings({ ...$audioSettings, ...patch }).catch(mediaError);
  onMount(() => {
    void reload();
    navigator.mediaDevices?.addEventListener('devicechange', reload);
    return () =>
      navigator.mediaDevices?.removeEventListener('devicechange', reload);
  });
</script>

<div class="modal-backdrop">
  <dialog open class="modal" aria-modal="true" aria-labelledby="device-title">
    <div class="modal-title">
      <h2 id="device-title">Dispositivos e qualidade</h2>
      <button aria-label="Fechar" onclick={close}><X /></button>
    </div>
    <p>As opções são aplicadas à chamada atual.</p>
    {#each [{ kind: 'audioinput', label: 'Microfone' }, { kind: 'audiooutput', label: 'Saída de áudio' }, { kind: 'videoinput', label: 'Câmera' }] as group}<label
        >{group.label}<select
          onchange={(e) =>
            switchDevice(
              group.kind as 'audioinput' | 'audiooutput' | 'videoinput',
              e.currentTarget.value,
            ).catch(mediaError)}
          ><option value="default">Padrão do sistema</option
          >{#each list.filter((d) => d.kind === group.kind && d.deviceId !== 'default') as d}<option
              value={d.deviceId}
              >{d.label || `${group.label} ${d.deviceId.slice(0, 6)}`}</option
            >{/each}</select
        ></label
      >{/each}
    <label
      >Qualidade do vídeo<select
        bind:value={level}
        onchange={() => quality(level).catch(mediaError)}
        ><option value="low">Econômica · 360p</option><option value="balanced"
          >Equilibrada · 720p</option
        ><option value="detail">Detalhada · 1080p</option></select
      ></label
    >
    <label
      >Supressão de ruído<select
        value={$audioSettings.noise}
        onchange={(e) =>
          updateAudio({ noise: e.currentTarget.value as NoiseMode })}
        ><option value="off">Desligada · voz sem filtro</option><option
          value="browser">Padrão do navegador</option
        ><option value="strong">Forte · RNNoise (IA)</option></select
      ></label
    >
    <label class="check"
      ><input
        type="checkbox"
        checked={$audioSettings.echo}
        onchange={(e) => updateAudio({ echo: e.currentTarget.checked })}
      />Cancelamento de eco</label
    >
    <label class="check"
      ><input
        type="checkbox"
        checked={$audioSettings.agc}
        onchange={(e) => updateAudio({ agc: e.currentTarget.checked })}
      />Ganho automático do microfone</label
    >
    <label class="check"
      ><input
        type="checkbox"
        checked={$audioSettings.shareAudio}
        onchange={(e) => updateAudio({ shareAudio: e.currentTarget.checked })}
      />Enviar som ao compartilhar a tela</label
    >
    <p class="hint">
      A supressão forte remove teclado, ventilador e ruídos de fundo, mas usa
      mais processamento. Sem fone, mantenha o cancelamento de eco ligado. As
      opções de áudio ficam salvas neste navegador. Para enviar o som da tela,
      marque "Compartilhar áudio" ao escolher o que compartilhar: no Windows o
      Chrome e o Edge enviam o som do sistema; no Linux e no macOS, só o de
      abas. Firefox e Safari não enviam som de tela.
    </p>
    {#if error}<p role="alert">{error}</p>{/if}
  </dialog>
</div>
