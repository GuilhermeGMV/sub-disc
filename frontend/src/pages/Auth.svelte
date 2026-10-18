<script lang="ts">
  import { Radio, ArrowRight, ShieldCheck } from '@lucide/svelte';
  import { post } from '../lib/api';
  import { t } from '../lib/i18n';
  import type { User } from '../lib/types';
  export let needsSetup = false;
  export let onlogin: (u: User) => void;
  let username = '',
    password = '',
    setupKey = '',
    invite = new URLSearchParams(location.hash.slice(1)).get('invite') || '';
  let register = !!invite;
  let busy = false,
    error = '';
  async function submit() {
    busy = true;
    error = '';
    try {
      const u = await post<User>(
        needsSetup ? '/setup' : register ? '/register' : '/login',
        {
          username,
          password,
          ...(needsSetup
            ? { setup_key: setupKey }
            : register
              ? { invite }
              : {}),
        },
      );
      history.replaceState(null, '', location.pathname);
      onlogin(u);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<div class="auth-shell">
  <div class="auth-story">
    <div class="brand">
      <span class="brand-mark"><Radio /></span>subdisc<span class="beta"
        >SELF-HOSTED</span
      >
    </div>
    <div>
      <span class="eyebrow">MENOS RUÍDO. MAIS CONVERSA.</span>
      <h1>O seu grupo.<br />O seu espaço.</h1>
      <p>Abra o microfone. Compartilhe uma ideia.<br />Fique mais um pouco.</p>
      <div class="sound-bars" aria-hidden="true">
        {#each [22, 38, 65, 40, 90, 60, 110, 76, 45, 80, 55, 28] as height}<i
            style={`height:${height}px`}
          ></i>{/each}
      </div>
    </div>
    <small><ShieldCheck size={16} /> Sua comunidade, no seu servidor.</small>
  </div>
  <main class="auth-form">
    <div class="mobile-brand">◉ subdisc</div>
    <span class="eyebrow"
      >{needsSetup
        ? 'BEM-VINDO À SUA INSTÂNCIA'
        : register
          ? 'TEM LUGAR PARA VOCÊ'
          : 'BOM TER VOCÊ POR AQUI'}</span
    >
    <h2>
      {needsSetup
        ? 'Vamos começar?'
        : register
          ? 'Entre para o grupo.'
          : 'Volte para a conversa.'}
    </h2>
    <p>
      {needsSetup
        ? 'Crie o administrador usando a chave definida no servidor.'
        : register
          ? 'Use seu convite para criar uma conta.'
          : 'Entre com a conta desta instância.'}
    </p>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <label
        >{t.username}<input
          aria-label={t.username}
          autocomplete="username"
          required
          minlength="3"
          maxlength="32"
          pattern="[A-Za-z0-9_-]+"
          bind:value={username}
          placeholder="seu_nome"
        /></label
      ><label
        >{t.password}<input
          aria-label={t.password}
          type="password"
          autocomplete={register || needsSetup
            ? 'new-password'
            : 'current-password'}
          required
          minlength="12"
          maxlength="128"
          bind:value={password}
          placeholder="Pelo menos 12 caracteres"
        /></label
      >{#if needsSetup}<label
          >{t.setupKey}<input
            aria-label={t.setupKey}
            type="password"
            required
            bind:value={setupKey}
          /></label
        >{:else if register}<label
          >{t.invite}<input
            aria-label={t.invite}
            required
            bind:value={invite}
          /></label
        >{/if}{#if error}<div class="error" role="alert">
          {error}
        </div>{/if}<button class="primary auth-submit" disabled={busy}
        >{busy
          ? 'Aguarde…'
          : needsSetup
            ? t.setup
            : register
              ? t.register
              : t.login}<ArrowRight size={18} /></button
      >
    </form>
    {#if !needsSetup}<button
        class="text-button"
        onclick={() => {
          register = !register;
          error = '';
        }}>{register ? 'Já tenho uma conta' : 'Tenho um convite'}</button
      >{/if}<small class="auth-note"
      >Privado por natureza. Sem contas externas.</small
    >
  </main>
</div>
