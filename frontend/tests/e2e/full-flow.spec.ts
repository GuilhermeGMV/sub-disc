import { test, expect, type Page } from '@playwright/test';
async function credentials(page: Page, username: string) {
  await page.getByLabel('Nome de usuário', { exact: true }).fill(username);
  await page.getByLabel('Senha', { exact: true }).fill('strong password 123');
}
async function instrument(page: Page, screenAudio = false) {
  await page.addInitScript((screenAudio) => {
    if (screenAudio) {
      // Only capture is synthetic. Encoding, signaling, SFU and playback remain real.
      const display = navigator.mediaDevices.getDisplayMedia.bind(
        navigator.mediaDevices,
      );
      Object.assign(window, { testScreenAudioEnabled: true });
      navigator.mediaDevices.getDisplayMedia = async (options) => {
        const stream = await display(options);
        for (const track of stream.getAudioTracks()) {
          track.stop();
          stream.removeTrack(track);
        }
        if (
          !options?.audio ||
          !(window as unknown as { testScreenAudioEnabled: boolean })
            .testScreenAudioEnabled
        )
          return stream;
        const context = new AudioContext({ sampleRate: 48000 });
        const left = context.createOscillator();
        const right = context.createOscillator();
        left.frequency.value = 440;
        right.frequency.value = 880;
        const stereo = context.createChannelMerger(2);
        left.connect(stereo, 0, 0);
        right.connect(stereo, 0, 1);
        const level = context.createGain();
        level.gain.value = 0.05;
        const destination = context.createMediaStreamDestination();
        stereo.connect(level).connect(destination);
        const audio = destination.stream.getAudioTracks()[0];
        const stop = audio.stop.bind(audio);
        let stopped = false;
        audio.stop = () => {
          if (stopped) return;
          stopped = true;
          stop();
          left.stop();
          right.stop();
          void context.close();
        };
        stream
          .getVideoTracks()[0]
          .addEventListener('ended', () => audio.stop(), { once: true });
        stream.addTrack(audio);
        left.start();
        right.start();
        await context.resume();
        return stream;
      };
    }
    const NativeSocket = window.WebSocket;
    const sockets: WebSocket[] = [];
    Object.assign(window, { testSockets: sockets });
    window.WebSocket = class extends NativeSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        sockets.push(this);
      }
    };
    const Native = window.RTCPeerConnection;
    const peers: RTCPeerConnection[] = [];
    Object.assign(window, { testPeers: peers });
    window.RTCPeerConnection = class extends Native {
      constructor(config?: RTCConfiguration) {
        super(config);
        peers.push(this);
      }
    };
  }, screenAudio);
}
async function received(page: Page, kind: string) {
  return page.evaluate(async (kind) => {
    let bytes = 0;
    for (const pc of (window as unknown as { testPeers: RTCPeerConnection[] })
      .testPeers) {
      if (pc.connectionState === 'closed') continue;
      const stats = await pc.getStats();
      stats.forEach((r) => {
        if (r.type === 'inbound-rtp' && r.kind === kind)
          bytes += r.bytesReceived || 0;
      });
    }
    return bytes;
  }, kind);
}
async function energy(page: Page) {
  return page.evaluate(async () => {
    let total = 0;
    for (const pc of (window as unknown as { testPeers: RTCPeerConnection[] })
      .testPeers) {
      if (pc.connectionState === 'closed') continue;
      (await pc.getStats()).forEach((stat) => {
        if (stat.type === 'inbound-rtp' && stat.kind === 'audio')
          total += stat.totalAudioEnergy || 0;
      });
    }
    return total;
  });
}
async function mediaMetrics(page: Page) {
  return page.evaluate(async () => {
    const roundTrips: number[] = [];
    let audioDelay = 0;
    let audioSamples = 0;
    let frames = 0;
    for (const pc of (window as unknown as { testPeers: RTCPeerConnection[] })
      .testPeers) {
      if (pc.connectionState !== 'connected') continue;
      (await pc.getStats()).forEach((stat) => {
        if (
          stat.type === 'candidate-pair' &&
          stat.state === 'succeeded' &&
          typeof stat.currentRoundTripTime === 'number'
        )
          roundTrips.push(stat.currentRoundTripTime * 1000);
        if (stat.type === 'inbound-rtp' && stat.kind === 'audio') {
          audioDelay += stat.jitterBufferDelay ?? 0;
          audioSamples += stat.jitterBufferEmittedCount ?? 0;
        }
        if (stat.type === 'inbound-rtp' && stat.kind === 'video')
          frames += stat.framesDecoded ?? 0;
      });
    }
    return {
      roundTripsMs: roundTrips,
      audioJitterBufferMs: audioSamples
        ? (audioDelay / audioSamples) * 1000
        : null,
      decodedVideoFrames: frames,
    };
  });
}
async function activeVideoLayers(page: Page) {
  return page.evaluate(() =>
    (window as unknown as { testPeers: RTCPeerConnection[] }).testPeers
      .filter((pc) => pc.connectionState !== 'closed')
      .flatMap((pc) => pc.getSenders())
      .filter((s) => s.track?.kind === 'video')
      .flatMap((s) => s.getParameters().encodings)
      .filter((e) => e.active)
      .map((e) => e.rid),
  );
}
const cameraWidth = (page: Page) =>
  page
    .locator('[data-source="camera"] video')
    .evaluate((video: HTMLVideoElement) => video.videoWidth);
test('dois amigos: cadastro, chat, WebSocket, áudio, câmera, tela e controles', async ({
  browser,
  page,
}) => {
  await instrument(page, true);
  await page.goto('/');
  await credentials(page, 'admin');
  await page.getByLabel('Chave de instalação').fill('a-safe-setup-test-key');
  await page.getByRole('button', { name: 'Criar minha instância' }).click();
  await expect(
    page.getByRole('heading', { name: 'geral', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Convidar amigos' }).click();
  await page
    .getByRole('button', { name: 'Gerar convite', exact: true })
    .click();
  const invite = await page
    .getByLabel('Link do convite', { exact: true })
    .inputValue();
  await page.getByRole('button', { name: 'Fechar', exact: true }).click();
  // Exercise login independently of registration's automatic session.
  await page.getByRole('button', { name: 'Sair da conta' }).click();
  await credentials(page, 'admin');
  await page.getByRole('button', { name: 'Entrar', exact: true }).click();
  const ctx = await browser.newContext({
    permissions: ['microphone', 'camera'],
  });
  const friend = await ctx.newPage();
  await instrument(friend);
  await friend.goto(invite);
  await credentials(friend, 'amigo');
  await friend
    .getByRole('button', { name: 'Criar conta', exact: true })
    .click();
  await expect(
    friend.getByRole('heading', { name: 'geral', exact: true }),
  ).toBeVisible();
  await page
    .getByLabel('Mensagem', { exact: true })
    .fill('Olá, chegou em tempo real!');
  await page.getByRole('button', { name: 'Enviar', exact: true }).click();
  await expect(
    friend
      .getByTestId('message')
      .filter({ hasText: 'Olá, chegou em tempo real!' }),
  ).toBeVisible();
  await friend
    .getByLabel('Mensagem', { exact: true })
    .fill('<script>seguro</script>');
  await friend.getByRole('button', { name: 'Enviar', exact: true }).click();
  await expect(
    page.getByTestId('message').filter({ hasText: '<script>seguro</script>' }),
  ).toBeVisible();
  await ctx.setOffline(true);
  await page
    .getByLabel('Mensagem', { exact: true })
    .fill('Mensagem durante a reconexão');
  await page.getByRole('button', { name: 'Enviar', exact: true }).click();
  await ctx.setOffline(false);
  await expect(
    friend
      .getByTestId('message')
      .filter({ hasText: 'Mensagem durante a reconexão' }),
  ).toBeVisible({ timeout: 30000 });
  await page
    .getByRole('button', { name: 'sala de estar', exact: true })
    .click();
  await friend.getByRole('button', { name: /^sala de estar/ }).click();
  await expect(page.getByTestId('voice-status')).toHaveText(
    'Chamada conectada',
  );
  await expect(friend.getByTestId('voice-status')).toHaveText(
    'Chamada conectada',
  );
  await expect(page.getByTestId('participant')).toHaveCount(2);
  await expect(friend.getByTestId('participant')).toHaveCount(2);
  await expect.poll(() => received(page, 'audio')).toBeGreaterThan(500);
  await expect.poll(() => received(friend, 'audio')).toBeGreaterThan(500);
  await expect.poll(() => energy(page)).toBeGreaterThan(0);
  await expect.poll(() => energy(friend)).toBeGreaterThan(0);
  await expect
    .poll(() => page.locator('.participant-card.speaking').count())
    .toBeGreaterThan(0);
  // Audio elements must survive switching from the call view to text chat.
  const audioBefore = await received(page, 'audio');
  await page.getByRole('button', { name: 'geral', exact: true }).click();
  await expect(page.locator('audio')).toHaveCount(1);
  await expect
    .poll(() => received(page, 'audio'))
    .toBeGreaterThan(audioBefore + 500);
  await expect
    .poll(() =>
      page
        .locator('audio')
        .evaluateAll((elements) =>
          elements.every((e) => !(e as HTMLAudioElement).paused),
        ),
    )
    .toBe(true);
  await page.getByRole('button', { name: /^sala de estar/ }).click();
  await page
    .getByRole('button', { name: 'Silenciar microfone', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Ativar microfone', exact: true }),
  ).toBeVisible();
  await expect(
    friend.getByTestId('participant').filter({ hasText: 'admin' }),
  ).toContainText('Silenciado');
  await page
    .getByRole('button', { name: 'Ativar microfone', exact: true })
    .click();
  await friend
    .getByRole('button', { name: 'Silenciar áudio recebido' })
    .click();
  await expect(
    friend.getByRole('button', { name: 'Silenciar áudio recebido' }),
  ).toHaveAttribute('aria-pressed', 'true');
  await friend
    .getByRole('button', { name: 'Silenciar áudio recebido' })
    .click();
  await page
    .getByRole('button', { name: 'Ativar câmera', exact: true })
    .click();
  await expect(friend.locator('[data-source="camera"] video')).toBeVisible();
  await expect.poll(() => received(friend, 'video')).toBeGreaterThan(1000);
  await expect
    .poll(async () => (await mediaMetrics(friend)).decodedVideoFrames)
    .toBeGreaterThan(2);
  // Simulcast: each receiver selects one layer for the displayed size, and the
  // sender only encodes layers that somebody receives.
  expect(
    await page.evaluate(() =>
      (window as unknown as { testPeers: RTCPeerConnection[] }).testPeers
        .filter((pc) => pc.connectionState !== 'closed')
        .flatMap((pc) => pc.getSenders())
        .filter((s) => s.track?.kind === 'video')
        .flatMap((s) => s.getParameters().encodings.map((e) => e.rid)),
    ),
  ).toEqual(['l', 'm', 'h']);
  await expect.poll(() => activeVideoLayers(page)).toEqual(['h']);
  await expect.poll(() => cameraWidth(friend)).toBe(1280);
  await friend.setViewportSize({ width: 390, height: 844 });
  await expect
    .poll(() => activeVideoLayers(page), { timeout: 30000 })
    .toEqual(['m']);
  await expect.poll(() => cameraWidth(friend), { timeout: 30000 }).toBe(640);
  await friend.setViewportSize({ width: 1280, height: 720 });
  await expect
    .poll(() => activeVideoLayers(page), { timeout: 30000 })
    .toEqual(['h']);
  await expect.poll(() => cameraWidth(friend), { timeout: 30000 }).toBe(1280);
  // Video that is no longer displayed is paused at the SFU and stops encoding.
  await friend.getByRole('button', { name: 'geral', exact: true }).click();
  await expect
    .poll(() => activeVideoLayers(page), { timeout: 30000 })
    .toEqual([]);
  await friend.getByRole('button', { name: /^sala de estar/ }).click();
  await expect
    .poll(() => activeVideoLayers(page), { timeout: 30000 })
    .toEqual(['h']);
  await expect
    .poll(
      () =>
        friend
          .locator('[data-source="camera"] video')
          .evaluate(
            (video: HTMLVideoElement) =>
              video.getVideoPlaybackQuality().totalVideoFrames,
          ),
      { timeout: 30000 },
    )
    .toBeGreaterThan(2);
  await page
    .getByRole('button', { name: 'Compartilhar tela', exact: true })
    .click();
  await expect(
    friend.locator('[data-source="screen_share"] video'),
  ).toBeVisible();
  const bitrates = await page.evaluate(() =>
    (window as unknown as { testPeers: RTCPeerConnection[] }).testPeers
      .filter((pc) => pc.connectionState !== 'closed')
      .flatMap((pc) =>
        pc
          .getSenders()
          .filter((s) => s.track?.kind === 'video')
          .flatMap((s) =>
            s.getParameters().encodings.map((encoding) => encoding.maxBitrate),
          ),
      ),
  );
  expect(bitrates).toEqual(expect.arrayContaining([1700000, 2500000]));
  // A distinct stereo test tone verifies the screen audio track through the SFU.
  await expect(
    friend.locator('[data-source="screen_share_audio"] audio'),
  ).toHaveCount(1);
  await expect
    .poll(() =>
      friend.evaluate(async () => {
        const element = document.querySelector<HTMLAudioElement>(
          '[data-source="screen_share_audio"] audio',
        );
        const track = (
          element?.srcObject as MediaStream | null
        )?.getAudioTracks()[0];
        let energy = 0;
        for (const pc of (
          window as unknown as { testPeers: RTCPeerConnection[] }
        ).testPeers) {
          if (pc.connectionState !== 'connected') continue;
          for (const receiver of pc.getReceivers()) {
            if (receiver.track.id !== track?.id) continue;
            (await receiver.getStats()).forEach((stat) => {
              if (stat.type === 'inbound-rtp' && stat.kind === 'audio')
                energy += stat.totalAudioEnergy ?? 0;
            });
          }
        }
        return energy;
      }),
    )
    .toBeGreaterThan(0);
  await page
    .getByRole('button', { name: 'Parar de enviar o som da tela', exact: true })
    .click();
  await expect
    .poll(() =>
      friend
        .locator('[data-source="screen_share_audio"] audio')
        .evaluate(
          (element: HTMLAudioElement) =>
            (element.srcObject as MediaStream).getAudioTracks()[0].enabled,
        ),
    )
    .toBe(false);
  await expect(
    page.getByRole('button', { name: 'Silenciar microfone', exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Enviar o som da tela', exact: true })
    .click();
  await expect
    .poll(() =>
      friend
        .locator('[data-source="screen_share_audio"] audio')
        .evaluate(
          (element: HTMLAudioElement) =>
            (element.srcObject as MediaStream).getAudioTracks()[0].enabled,
        ),
    )
    .toBe(true);
  const screenVolume = friend.getByRole('slider', {
    name: 'Volume do som da tela de admin',
  });
  await expect(screenVolume).toBeVisible();
  {
    const voiceVolume = friend.getByRole('slider', { name: 'Volume de admin' });
    const before = await voiceVolume.inputValue();
    await screenVolume.evaluate((input: HTMLInputElement) => {
      input.value = '0.25';
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await expect(screenVolume).toHaveValue('0.25');
    await expect(voiceVolume).toHaveValue(before);
    await expect(
      friend.getByRole('slider', { name: 'Volume da tela de admin' }),
    ).toHaveValue('0.25');
  }
  await page
    .getByRole('button', { name: 'Parar compartilhamento', exact: true })
    .click();
  await expect(
    friend.locator('[data-source="screen_share"] video'),
  ).toHaveCount(0);
  await expect(
    friend.locator('[data-source="screen_share_audio"] audio'),
  ).toHaveCount(0);
  // Also preserve the UI for a platform that returns video without audio.
  await page.evaluate(() =>
    Object.assign(window, { testScreenAudioEnabled: false }),
  );
  await page
    .getByRole('button', { name: 'Compartilhar tela', exact: true })
    .click();
  await expect(friend.getByText('Sem áudio', { exact: true })).toBeVisible();
  await expect(
    friend.locator('[data-source="screen_share_audio"] audio'),
  ).toHaveCount(0);
  await page
    .getByRole('button', { name: 'Parar compartilhamento', exact: true })
    .click();
  // Interrupt real signaling and force fresh ICE transports while keeping capture alive.
  await ctx.setOffline(true);
  await friend.evaluate(() => {
    for (const ws of (window as unknown as { testSockets: WebSocket[] })
      .testSockets)
      if (ws.url.includes('/media/ws')) ws.close();
    for (const pc of (window as unknown as { testPeers: RTCPeerConnection[] })
      .testPeers)
      if (pc.connectionState !== 'closed') pc.close();
  });
  await expect(friend.getByTestId('voice-status')).toHaveText('Reconectando…');
  await ctx.setOffline(false);
  await expect(friend.getByTestId('voice-status')).toHaveText(
    'Chamada conectada',
    { timeout: 30000 },
  );
  await expect
    .poll(() => energy(friend), { timeout: 30000 })
    .toBeGreaterThan(0);
  await expect.poll(() => energy(page), { timeout: 30000 }).toBeGreaterThan(0);
  await expect
    .poll(
      () =>
        friend
          .locator('[data-source="camera"] video')
          .evaluateAll((elements) =>
            elements.some(
              (e) =>
                (e as HTMLVideoElement).getVideoPlaybackQuality()
                  .totalVideoFrames > 2,
            ),
          ),
      { timeout: 30000 },
    )
    .toBe(true);
  // Retry failed publish offers without reopening capture or losing quality limits.
  const oldCameraId = await friend
    .locator('[data-source="camera"] video')
    .evaluate(
      (video: HTMLVideoElement) =>
        (video.srcObject as MediaStream).getVideoTracks()[0].id,
    );
  let rejectedOffers = 0;
  await page.route('**/api/media/publish', (route) => {
    if (rejectedOffers++ < 2)
      return route.fulfill({
        status: 503,
        contentType: 'application/json',
        body: '{"error":"temporary outage"}',
      });
    return route.continue();
  });
  await page.evaluate(() => {
    for (const pc of (window as unknown as { testPeers: RTCPeerConnection[] })
      .testPeers)
      if (
        pc.connectionState !== 'closed' &&
        pc.getSenders().some((s) => s.track?.kind === 'video')
      )
        pc.close();
  });
  await expect
    .poll(() => rejectedOffers, { timeout: 30000 })
    .toBeGreaterThanOrEqual(3);
  await expect
    .poll(
      () =>
        friend.locator('[data-source="camera"] video').evaluateAll(
          (videos, oldId) =>
            videos.some((video) => {
              const v = video as HTMLVideoElement;
              return (
                (v.srcObject as MediaStream)?.getVideoTracks()[0]?.id !==
                  oldId && v.getVideoPlaybackQuality().totalVideoFrames > 2
              );
            }),
          oldCameraId,
        ),
      { timeout: 30000 },
    )
    .toBe(true);
  const recoveredBitrates = await page.evaluate(() =>
    (window as unknown as { testPeers: RTCPeerConnection[] }).testPeers
      .filter((pc) => pc.connectionState !== 'closed')
      .flatMap((pc) =>
        pc
          .getSenders()
          .filter((s) => s.track?.kind === 'video')
          .flatMap((s) =>
            s.getParameters().encodings.map((encoding) => encoding.maxBitrate),
          ),
      ),
  );
  expect(recoveredBitrates).toContain(1700000);
  await page.unroute('**/api/media/publish');
  const metrics = await mediaMetrics(friend);
  expect(metrics.roundTripsMs.length).toBeGreaterThan(0);
  expect(Math.max(...metrics.roundTripsMs)).toBeLessThan(1000);
  console.log('Local WebRTC statistics:', metrics);
  const tab = await page.context().newPage();
  await tab.goto('/');
  await tab.getByRole('button', { name: /^sala de estar/ }).click();
  await expect(tab.getByRole('alert')).toContainText('outra aba');
  await tab.close();
  await page.screenshot({
    path: 'test-results/call-desktop.png',
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Sair da chamada' }).click();
  await expect(friend.getByTestId('participant')).toHaveCount(1);
  await friend.getByRole('button', { name: 'Sair da chamada' }).click();
  await page.getByRole('button', { name: 'geral', exact: true }).click();
  await page.screenshot({
    path: 'test-results/chat-desktop.png',
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole('button', { name: 'Abrir navegação' }),
  ).toBeVisible();
  for (const selector of ['.send-button', '.connection', '.call-controls']) {
    const box = await page.locator(selector).boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  }
  await page.screenshot({
    path: 'test-results/chat-mobile.png',
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await ctx.close();
});
