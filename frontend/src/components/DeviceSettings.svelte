<script lang="ts">
  import { onMount } from 'svelte';
  import { X } from '@lucide/svelte';
  import { devices, switchDevice, quality, mediaError } from '../stores/media';
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
    <p class="hint">
      Cancelamento de eco e supressão de ruído são solicitados ao navegador.
      Saída de áudio e áudio de tela dependem do navegador e sistema.
    </p>
    {#if error}<p role="alert">{error}</p>{/if}
  </dialog>
</div>
