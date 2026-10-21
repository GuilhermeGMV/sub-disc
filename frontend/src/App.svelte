<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    Radio,
    Hash,
    Volume2,
    Plus,
    Mic,
    MicOff,
    Headphones,
    HeadphoneOff,
    Video,
    VideoOff,
    Monitor,
    MonitorSpeaker,
    MonitorOff,
    PhoneOff,
    Settings,
    LogOut,
    Send,
    Users,
    Menu,
    X,
    UserPlus,
    Pencil,
    Trash2,
    Copy,
    ChevronRight,
  } from '@lucide/svelte';
  import AudioOutput from './components/AudioOutput.svelte';
  import Auth from './pages/Auth.svelte';
  import Call from './components/Call.svelte';
  import DeviceSettings from './components/DeviceSettings.svelte';
  import { api, post, ApiError } from './lib/api';
  import { t } from './lib/i18n';
  import {
    parseEvent,
    mergeMessages,
    type User,
    type Channel,
    type ChatMessage,
    type Snapshot,
  } from './lib/types';
  import {
    media,
    joinVoice,
    leaveVoice,
    toggleMic,
    toggleCamera,
    toggleScreen,
    toggleScreenAudio,
    toggleDeafen,
    setPresenceSender,
    mediaError,
  } from './stores/media';
  let loading = true,
    needsSetup = false,
    user: User | null = null,
    error = '';
  let snapshot: Snapshot = { users: [], channels: [], online: [], voice: [] };
  let selected: Channel | null = null,
    messages: ChatMessage[] = [],
    draft = '',
    sending = false,
    historyLoading = false,
    hasMore = true;
  let connection = 'connecting',
    ws: WebSocket | null = null,
    retry: ReturnType<typeof setTimeout>,
    retryCount = 0,
    stopped = false;
  let scrollArea: HTMLDivElement;
  let mobileNav = false,
    showMembers = true,
    showDevices = false,
    quality = 'balanced';
  let adminPanel = location.pathname === '/admin',
    inviteUrl = '',
    adminBusy = false,
    channelName = '',
    channelKind = 'text',
    editing: Channel | null = null;
  let accountName = '',
    accountPassword = '',
    accountRole = 'member',
    accountSearch = '';
  let editingAccount: User | null = null;
  $: voiceChannel = snapshot.channels.find((c) => c.id === $media.channelId);
  $: onlineUsers = snapshot.users.filter((u) => snapshot.online.includes(u.id));
  $: offlineUsers = snapshot.users.filter(
    (u) => !snapshot.online.includes(u.id),
  );
  async function boot() {
    loading = true;
    error = '';
    try {
      const status = await api<{ needs_setup: boolean }>('/setup');
      needsSetup = status.needs_setup;
      if (!needsSetup) {
        try {
          user = await api<User>('/me');
          await initialize();
        } catch (e) {
          if (!(e instanceof ApiError && e.status === 401)) throw e;
          user = null;
        }
      }
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }
  async function initialize() {
    snapshot = await api<Snapshot>('/state');
    selected = snapshot.channels.find((c) => c.kind === 'text') || null;
    if (selected) await loadHistory(true);
    connect();
  }
  async function loggedIn(u: User) {
    user = u;
    needsSetup = false;
    try {
      await initialize();
    } catch (e) {
      error = (e as Error).message;
    }
  }
  function connect() {
    if (stopped || !user) return;
    clearTimeout(retry);
    ws?.close();
    const socket = new WebSocket(
      `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/ws`,
    );
    ws = socket;
    connection = 'connecting';
    socket.onopen = () => {
      if (ws !== socket) return;
      connection = 'connected';
      retryCount = 0;
      setPresenceSender((data) => {
        if (socket.readyState === WebSocket.OPEN)
          socket.send(JSON.stringify(data));
      });
    };
    socket.onmessage = (e) => {
      const event = parseEvent(e.data);
      if (!event) return;
      if (event.type === 'snapshot') {
        snapshot = event.data;
        reconcileSelection();
        if (selected?.kind === 'text') void loadHistory(true).catch(report);
      }
      if (event.type === 'voice_reset') void leaveVoice().catch(mediaError);
      if (event.type === 'presence')
        snapshot = { ...snapshot, online: event.online, voice: event.voice };
      if (
        event.type === 'message' &&
        event.message.channel_id === selected?.id
      ) {
        const nearBottom =
          !scrollArea ||
          scrollArea.scrollHeight -
            scrollArea.scrollTop -
            scrollArea.clientHeight <
            140;
        messages = mergeMessages(messages, [event.message]);
        if (nearBottom) void toBottom();
      }
      if (event.type === 'channels_changed' || event.type === 'users_changed')
        void api<Snapshot>('/state')
          .then((data) => {
            snapshot = data;
            reconcileSelection();
          })
          .catch(report);
    };
    socket.onclose = () => {
      if (ws !== socket || stopped || !user) return;
      connection = 'reconnecting';
      retry = setTimeout(
        async () => {
          try {
            user = await api<User>('/me');
            connect();
          } catch (e) {
            if (e instanceof ApiError && e.status === 401) {
              user = null;
              await leaveVoice().catch(mediaError);
            } else connect();
          }
        },
        Math.min(15000, 1000 * 2 ** retryCount++) + Math.random() * 500,
      );
    };
  }
  function reconcileSelection() {
    if (user) user = snapshot.users.find((u) => u.id === user!.id) || user;
    if (selected) {
      selected = snapshot.channels.find((c) => c.id === selected!.id) || null;
    }
    if (!selected) {
      selected = snapshot.channels.find((c) => c.kind === 'text') || null;
      messages = [];
      if (selected) void loadHistory(true).catch(report);
    }
  }
  function report(e: unknown) {
    error = e instanceof Error ? e.message : String(e);
  }
  async function toBottom() {
    await tick();
    if (scrollArea) scrollArea.scrollTop = scrollArea.scrollHeight;
  }
  async function loadHistory(reset: boolean, before?: number) {
    if (!selected || selected.kind !== 'text') return;
    const id = selected.id;
    const lastId = messages.at(-1)?.id ?? 0;
    historyLoading = true;
    try {
      const data = await api<ChatMessage[]>(
        `/channels/${id}/messages${before ? `?before=${before}` : ''}`,
      );
      if (selected?.id !== id) return;
      messages = reset
        ? mergeMessages(
            data,
            messages.filter((m) => m.channel_id === id && m.id > lastId),
          )
        : mergeMessages(messages, data);
      if (reset || before) hasMore = data.length === 50;
      if (reset) void toBottom();
    } finally {
      historyLoading = false;
    }
  }
  async function choose(c: Channel) {
    if (
      c.kind === 'voice' &&
      ['connecting', 'disconnecting'].includes($media.status)
    )
      return;
    selected = c;
    mobileNav = false;
    messages = [];
    hasMore = true;
    error = '';
    if (c.kind === 'text') await loadHistory(true);
    else if ($media.channelId !== c.id) {
      if ($media.room) await leaveVoice();
      await joinVoice(c.id);
    }
  }
  async function send() {
    if (!draft.trim() || !selected || sending) return;
    const id = selected.id,
      content = draft;
    sending = true;
    try {
      const msg = await post<ChatMessage>(`/channels/${id}/messages`, {
        content,
      });
      if (selected?.id === id) {
        messages = mergeMessages(messages, [msg]);
        draft = '';
        await toBottom();
      }
    } catch (e) {
      report(e);
    } finally {
      sending = false;
    }
  }
  async function logout() {
    try {
      await leaveVoice();
      await post('/logout');
      user = null;
      ws?.close();
      clearTimeout(retry);
      snapshot = { users: [], channels: [], online: [], voice: [] };
    } catch (e) {
      report(e);
    }
  }
  async function createInvite() {
    adminBusy = true;
    try {
      const r = await post<{ url: string }>('/invites', { hours: 24 });
      inviteUrl = r.url;
    } catch (e) {
      report(e);
    } finally {
      adminBusy = false;
    }
  }
  async function saveAccount() {
    adminBusy = true;
    try {
      await api(editingAccount ? `/users/${editingAccount.id}` : '/users', {
        method: editingAccount ? 'PATCH' : 'POST',
        body: JSON.stringify({
          username: accountName,
          password: accountPassword,
          role: accountRole,
        }),
      });
      accountName = '';
      accountPassword = '';
      accountRole = 'member';
      editingAccount = null;
      snapshot = await api('/state');
    } catch (e) {
      report(e);
    } finally {
      adminBusy = false;
    }
  }
  async function deleteAccount(account: User) {
    if (
      !confirm(
        `Apagar a conta de ${account.username}, suas mensagens e convites? Esta ação é permanente.`,
      )
    )
      return;
    adminBusy = true;
    try {
      await api(`/users/${account.id}`, { method: 'DELETE' });
      snapshot = await api('/state');
    } catch (e) {
      report(e);
    } finally {
      adminBusy = false;
    }
  }
  async function disconnectAccount(account: User) {
    adminBusy = true;
    try {
      await post(`/users/${account.id}/disconnect`);
      snapshot = await api('/state');
    } catch (e) {
      report(e);
    } finally {
      adminBusy = false;
    }
  }
  async function saveChannel() {
    adminBusy = true;
    try {
      const body = { name: channelName, kind: channelKind };
      if (editing)
        await api(`/channels/${editing.id}`, {
          method: 'PATCH',
          body: JSON.stringify(body),
        });
      else await post('/channels', body);
      channelName = '';
      editing = null;
      snapshot = await api('/state');
      reconcileSelection();
    } catch (e) {
      report(e);
    } finally {
      adminBusy = false;
    }
  }
  async function deleteChannel(c: Channel) {
    if (
      !confirm(`Excluir “${c.name}” e seu histórico? Esta ação é permanente.`)
    )
      return;
    try {
      await api(`/channels/${c.id}`, { method: 'DELETE' });
      snapshot = await api('/state');
      reconcileSelection();
    } catch (e) {
      report(e);
    }
  }
  function shortcut(e: KeyboardEvent) {
    if (e.ctrlKey && e.shiftKey) {
      if (e.code === 'KeyM') {
        e.preventDefault();
        void toggleMic().catch(mediaError);
      }
      if (e.code === 'KeyD') {
        e.preventDefault();
        toggleDeafen();
      }
    }
    if (e.key === 'Escape') {
      showDevices = false;
      adminPanel = false;
      mobileNav = false;
    }
  }
  onMount(() => {
    void boot();
    window.addEventListener('keydown', shortcut);
    return () => {
      stopped = true;
      clearTimeout(retry);
      ws?.close();
      void leaveVoice();
      window.removeEventListener('keydown', shortcut);
    };
  });
</script>

{#if loading}<div class="loading-screen">
    <Radio size={40} />
    <p>Abrindo seu espaço…</p>
  </div>
{:else if !user}{#if error}<div class="boot-error error" role="alert">
      {error}<button onclick={boot}>Tentar novamente</button>
    </div>{/if}<Auth {needsSetup} onlogin={loggedIn} />
{:else}
  <AudioOutput />
  <div class="workspace">
    <nav class="server-rail" aria-label="Instância">
      <button
        class="server-logo"
        aria-label="Subdisc"
        onclick={() => (mobileNav = !mobileNav)}><Radio size={26} /></button
      ><span class="rail-line"></span><span class="rail-label">SEU ESPAÇO</span
      ><span class="rail-bottom">S.</span>
    </nav>
    <aside class:mobile-open={mobileNav} class="sidebar">
      <div class="server-title">
        <div><strong>subdisc</strong><small>Um espaço entre amigos</small></div>
        <button
          class="mobile-only"
          aria-label="Fechar navegação"
          onclick={() => (mobileNav = false)}><X size={18} /></button
        >
      </div>
      <div class="channel-scroll">
        <div class="section-title">
          {t.textChannels}{#if user.role === 'admin'}<button
              aria-label="Adicionar canal"
              onclick={() => {
                adminPanel = true;
                channelKind = 'text';
              }}><Plus size={16} /></button
            >{/if}
        </div>
        {#each snapshot.channels.filter((c) => c.kind === 'text') as c}<button
            class:active={selected?.id === c.id}
            class="channel"
            onclick={() => choose(c).catch(report)}
            ><Hash size={18} /><span>{c.name}</span
            >{#if selected?.id === c.id}<span class="channel-dot"
              ></span>{/if}</button
          >{/each}
        <div class="section-title voice-title">
          {t.voiceChannels}{#if user.role === 'admin'}<button
              aria-label="Adicionar sala de voz"
              onclick={() => {
                adminPanel = true;
                channelKind = 'voice';
              }}><Plus size={16} /></button
            >{/if}
        </div>
        {#each snapshot.channels.filter((c) => c.kind === 'voice') as c}<button
            class:active={selected?.id === c.id}
            class:joined={$media.channelId === c.id}
            class="channel"
            onclick={() => choose(c).catch(report)}
            ><Volume2 size={18} /><span>{c.name}</span><small
              >{snapshot.voice.filter((v) => v.channel_id === c.id).length ||
                ''}</small
            ></button
          >
          <div class="voice-members">
            {#each snapshot.voice.filter((v) => v.channel_id === c.id) as v}<div
              >
                <span class="tiny-avatar"
                  >{snapshot.users
                    .find((u) => u.id === v.user_id)
                    ?.username.slice(0, 1)
                    .toUpperCase()}</span
                ><span
                  >{snapshot.users.find((u) => u.id === v.user_id)
                    ?.username}</span
                >{#if v.muted}<MicOff size={12} />{/if}{#if v.screen}<Monitor
                    size={12}
                  />{/if}{#if v.camera}<Video size={12} />{/if}
              </div>{/each}
          </div>{/each}
      </div>
      {#if user.role === 'admin'}<button
          class="invite-button"
          onclick={() => (adminPanel = true)}
          ><UserPlus size={17} />Administração<ChevronRight size={15} /></button
        >{/if}
      <div class="self">
        <div class="avatar">
          {user.username.slice(0, 2).toUpperCase()}<i></i>
        </div>
        <div>
          <strong>{user.username}</strong><small
            >{user.role === 'admin' ? 'Administrador' : 'Membro'}</small
          >
        </div>
        <button title={t.logout} aria-label={t.logout} onclick={logout}
          ><LogOut size={17} /></button
        >
      </div>
    </aside>
    <main class="main">
      <header class="topbar">
        <button
          class="mobile-only"
          aria-label="Abrir navegação"
          onclick={() => (mobileNav = !mobileNav)}><Menu size={22} /></button
        >{#if selected?.kind === 'text'}<Hash size={23} />{:else}<Volume2
            size={23}
          />{/if}
        <h1>{selected?.name || 'Seu espaço'}</h1>
        <span class="header-divider"></span><span class="channel-subtitle"
          >{selected?.kind === 'text'
            ? 'As melhores conversas começam com um oi.'
            : 'Puxe uma cadeira.'}</span
        >
        <div class="topbar-end">
          <span class:healthy={connection === 'connected'} class="connection"
            ><i></i>{connection === 'connected'
              ? 'Online'
              : connection === 'reconnecting'
                ? 'Reconectando…'
                : 'Conectando…'}</span
          ><button
            aria-label="Mostrar participantes"
            aria-pressed={showMembers}
            onclick={() => (showMembers = !showMembers)}
            ><Users size={21} /></button
          >
        </div>
      </header>
      {#if error}<div class="error toast" role="alert">
          {error}<button aria-label="Fechar erro" onclick={() => (error = '')}
            ><X size={16} /></button
          >
        </div>{/if}
      {#if $media.error}<div class="error toast" role="alert">
          {$media.error}<button
            aria-label="Fechar aviso de mídia"
            onclick={() => media.update((s) => ({ ...s, error: '' }))}
            ><X size={16} /></button
          >
        </div>{/if}
      <div class="content-row">
        <section class="conversation">
          {#if selected?.kind === 'text'}<div
              class="message-scroll"
              bind:this={scrollArea}
              role="log"
              aria-label="Mensagens"
            >
              <div class="chat-welcome">
                <div class="welcome-hash"><Hash size={31} /></div>
                <span class="eyebrow">NOSSO PONTO DE ENCONTRO</span>
                <h2>Olá, #{selected.name}.</h2>
                <p>{t.emptyHint}</p>
              </div>
              {#if hasMore && messages.length}<button
                  class="load-more"
                  disabled={historyLoading}
                  onclick={() =>
                    loadHistory(false, messages[0].id).catch(report)}
                  >{historyLoading
                    ? 'Carregando…'
                    : 'Carregar mensagens anteriores'}</button
                >{/if}{#if historyLoading && !messages.length}<p class="muted">
                  Carregando mensagens…
                </p>{/if}
              {#each messages as m (m.id)}<article
                  class="message"
                  data-testid="message"
                >
                  <div class="avatar message-avatar">
                    {m.username.slice(0, 2).toUpperCase()}
                  </div>
                  <div>
                    <div class="message-meta">
                      <strong>{m.username}</strong><time
                        datetime={new Date(m.created_at * 1000).toISOString()}
                        title={new Date(m.created_at * 1000).toLocaleString(
                          'pt-BR',
                        )}
                        >{new Date(m.created_at * 1000).toLocaleTimeString(
                          'pt-BR',
                          { hour: '2-digit', minute: '2-digit' },
                        )}</time
                      >
                    </div>
                    <p>{m.content}</p>
                  </div>
                </article>{/each}
            </div>
            <div class="composer-wrap">
              <form
                class="composer"
                onsubmit={(e) => {
                  e.preventDefault();
                  send();
                }}
              >
                <textarea
                  aria-label="Mensagem"
                  bind:value={draft}
                  maxlength="4000"
                  rows="1"
                  placeholder={`Conversar em #${selected.name}`}
                  onkeydown={(e) => {
                    if (e.key === 'Enter' && !e.shiftKey) {
                      e.preventDefault();
                      void send();
                    }
                  }}></textarea><button
                  class="send-button"
                  aria-label={t.send}
                  disabled={!draft.trim() || sending}><Send size={19} /></button
                >
              </form>
              <div class="composer-hint">
                <span
                  ><kbd>Enter</kbd> para enviar · <kbd>Shift + Enter</kbd> para nova
                  linha</span
                ><span>{draft.length}/4000</span>
              </div>
            </div>
          {:else}<Call />{/if}
        </section>
        {#if showMembers}<aside class="members-panel">
            <div class="section-title">ONLINE — {onlineUsers.length}</div>
            {#each onlineUsers as member}<div class="member">
                <div class="avatar small">
                  {member.username.slice(0, 2).toUpperCase()}<i></i>
                </div>
                <div>
                  <strong>{member.username}</strong><small
                    >{snapshot.voice.some((v) => v.user_id === member.id)
                      ? 'Em chamada'
                      : member.role === 'admin'
                        ? 'Administrador'
                        : 'Por aqui'}</small
                  >
                </div>
              </div>{/each}{#if offlineUsers.length}<div
                class="section-title offline-title"
              >
                OFFLINE — {offlineUsers.length}
              </div>
              {#each offlineUsers as member}<div class="member offline">
                  <div class="avatar small">
                    {member.username.slice(0, 2).toUpperCase()}
                  </div>
                  <strong>{member.username}</strong>
                </div>{/each}{/if}
            <div class="private-note">
              <Radio size={22} /><strong>Só a nossa turma.</strong>
              <p>Conversas no seu servidor.<br />Do nosso jeito.</p>
            </div>
          </aside>{/if}
      </div>
      <footer class="call-bar">
        <div class="call-info">
          <span class:live={!!$media.room} class="call-symbol"
            ><Volume2 size={20} /></span
          >
          <div>
            <strong>{voiceChannel?.name || 'Tudo pronto para conversar'}</strong
            ><small
              >{$media.room
                ? $media.status === 'connected'
                  ? 'Voz conectada'
                  : $media.status === 'reconnecting'
                    ? 'Reconectando…'
                    : 'Conectando…'
                : 'Escolha uma sala de voz para entrar'}</small
            >
          </div>
        </div>
        <div class="call-controls">
          <button
            class:off={$media.muted}
            disabled={!$media.room}
            aria-label={$media.muted
              ? 'Ativar microfone'
              : 'Silenciar microfone'}
            aria-pressed={!$media.muted}
            title="Microfone · Ctrl+Shift+M"
            onclick={() => toggleMic().catch(mediaError)}
            >{#if $media.muted}<MicOff size={20} />{:else}<Mic
                size={20}
              />{/if}</button
          ><button
            class:off={$media.deafened}
            disabled={!$media.room}
            aria-label={t.deafen}
            aria-pressed={$media.deafened}
            title="Áudio recebido · Ctrl+Shift+D"
            onclick={toggleDeafen}
            >{#if $media.deafened}<HeadphoneOff size={20} />{:else}<Headphones
                size={20}
              />{/if}</button
          ><span class="control-divider"></span><button
            class:enabled={$media.camera}
            disabled={!$media.room}
            aria-label={$media.camera ? 'Desligar câmera' : 'Ativar câmera'}
            aria-pressed={$media.camera}
            onclick={() => toggleCamera().catch(mediaError)}
            >{#if $media.camera}<Video size={20} />{:else}<VideoOff
                size={20}
              />{/if}</button
          ><button
            class:enabled={$media.screen}
            disabled={!$media.room}
            aria-label={$media.screen
              ? 'Parar compartilhamento'
              : 'Compartilhar tela'}
            aria-pressed={$media.screen}
            onclick={() => toggleScreen(quality).catch(mediaError)}
            ><Monitor size={20} /></button
          >{#if $media.screenAudio !== 'none'}<button
              class:enabled={$media.screenAudio === 'on'}
              aria-label={$media.screenAudio === 'on'
                ? 'Parar de enviar o som da tela'
                : 'Enviar o som da tela'}
              aria-pressed={$media.screenAudio === 'on'}
              title="Som da tela"
              onclick={() => toggleScreenAudio().catch(mediaError)}
              >{#if $media.screenAudio === 'on'}<MonitorSpeaker
                  size={20}
                />{:else}<MonitorOff size={20} />{/if}</button
            >{/if}<button
            aria-label={t.settings}
            disabled={!$media.room}
            onclick={() => (showDevices = true)}><Settings size={19} /></button
          >{#if $media.room}<button
              class="hangup"
              aria-label={t.leave}
              onclick={() => leaveVoice().catch(mediaError)}
              ><PhoneOff size={20} /></button
            >{/if}
        </div>
      </footer>
    </main>
  </div>
{/if}
{#if showDevices}<DeviceSettings
    close={() => (showDevices = false)}
    bind:level={quality}
  />{/if}
{#if adminPanel && user?.role === 'admin'}<div class="modal-backdrop">
    <dialog
      open
      class="modal admin-modal"
      aria-modal="true"
      aria-labelledby="admin-title"
    >
      <div class="modal-title">
        <h2 id="admin-title">Administração</h2>
        <button aria-label="Fechar" onclick={() => (adminPanel = false)}
          ><X /></button
        >
      </div>
      <h3>Usuários</h3>
      <label
        >Buscar usuário<input
          aria-label="Buscar usuário"
          bind:value={accountSearch}
        /></label
      >
      <div class="admin-channels">
        {#each snapshot.users.filter( (u) => u.username.includes(accountSearch.toLowerCase()) ) as account}
          <div>
            <span>{account.username} · {account.role}</span>
            <button
              disabled={adminBusy}
              aria-label={`Editar ${account.username}`}
              onclick={() => {
                editingAccount = account;
                accountName = account.username;
                accountRole = account.role;
                accountPassword = '';
              }}><Pencil size={15} /></button
            >
            {#if snapshot.voice.some((v) => v.user_id === account.id)}
              <button
                disabled={adminBusy}
                aria-label={`Tirar ${account.username} da chamada`}
                onclick={() => disconnectAccount(account)}
                ><PhoneOff size={15} /></button
              >
            {/if}
            <button
              disabled={adminBusy}
              aria-label={`Apagar conta de ${account.username}`}
              onclick={() => deleteAccount(account)}
              ><Trash2 size={15} /></button
            >
          </div>
        {/each}
      </div>
      <h3>{editingAccount ? 'Editar usuário' : 'Criar usuário'}</h3>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void saveAccount();
        }}
      >
        <label
          >Usuário<input
            aria-label="Usuário da conta"
            required
            minlength="3"
            maxlength="32"
            pattern="[a-zA-Z0-9_-]+"
            bind:value={accountName}
          /></label
        >
        <label
          >{editingAccount
            ? 'Nova senha (vazia mantém a atual)'
            : 'Senha'}<input
            aria-label="Senha da conta"
            type="password"
            autocomplete="new-password"
            required={!editingAccount}
            minlength="12"
            maxlength="128"
            bind:value={accountPassword}
          /></label
        >
        <p>Alterar a senha encerra as sessões da conta.</p>
        <label
          >Role<select aria-label="Role da conta" bind:value={accountRole}
            ><option value="member">member</option><option value="admin"
              >admin</option
            ></select
          ></label
        >
        <button class="primary" disabled={adminBusy}>Salvar usuário</button>
        {#if editingAccount}<button
            type="button"
            onclick={() => {
              editingAccount = null;
              accountName = '';
              accountPassword = '';
              accountRole = 'member';
            }}>Cancelar edição</button
          >{/if}
      </form>
      <hr />
      <h3>Convide alguém</h3>
      <p>Um convite por pessoa, válido por 24 horas.</p>
      <button class="primary" disabled={adminBusy} onclick={createInvite}
        ><UserPlus size={17} />Gerar convite</button
      >{#if inviteUrl}<label
          >Link do convite
          <div class="copy-row">
            <input
              aria-label="Link do convite"
              readonly
              value={inviteUrl}
            /><button
              aria-label="Copiar convite"
              onclick={() =>
                navigator.clipboard.writeText(inviteUrl).catch(report)}
              ><Copy size={18} /></button
            >
          </div></label
        >{/if}
      <hr />
      <h3>{editing ? 'Renomear canal' : 'Criar canal'}</h3>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          saveChannel();
        }}
      >
        <label
          >Nome do canal<input
            aria-label="Nome do canal"
            required
            maxlength="48"
            bind:value={channelName}
          /></label
        ><label
          >Tipo<select
            aria-label="Tipo do canal"
            bind:value={channelKind}
            disabled={!!editing}
            ><option value="text">Texto</option><option value="voice"
              >Voz</option
            ></select
          ></label
        ><button class="primary" disabled={adminBusy}
          >{editing ? 'Salvar nome' : 'Criar canal'}</button
        >{#if editing}<button
            type="button"
            onclick={() => {
              editing = null;
              channelName = '';
            }}>Cancelar edição</button
          >{/if}
      </form>
      <div class="admin-channels">
        {#each snapshot.channels as c}<div>
            <span>{c.kind === 'text' ? '#' : '◉'} {c.name}</span><button
              aria-label={`Renomear ${c.name}`}
              onclick={() => {
                editing = c;
                channelName = c.name;
                channelKind = c.kind;
              }}><Pencil size={15} /></button
            ><button
              aria-label={`Excluir ${c.name}`}
              onclick={() => deleteChannel(c)}><Trash2 size={15} /></button
            >
          </div>{/each}
      </div>
      {#if error}<div class="error" role="alert">{error}</div>{/if}
    </dialog>
  </div>{/if}
