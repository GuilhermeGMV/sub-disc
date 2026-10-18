import { test, expect, type Page } from '@playwright/test';
async function credentials(page: Page, username: string) {
  await page.getByLabel('Nome de usuário', { exact: true }).fill(username);
  await page.getByLabel('Senha', { exact: true }).fill('strong password 123');
}
async function instrument(page: Page) {
  await page.addInitScript(() => {
    const Native = window.RTCPeerConnection;
    const peers: RTCPeerConnection[] = [];
    Object.assign(window, { testPeers: peers });
    window.RTCPeerConnection = class extends Native {
      constructor(config?: RTCConfiguration) {
        super(config);
        peers.push(this);
      }
    };
  });
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
test('dois amigos: cadastro, chat, WebSocket, áudio, câmera, tela e controles', async ({
  browser,
  page,
}) => {
  await instrument(page);
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
  await page
    .getByRole('button', { name: 'Compartilhar tela', exact: true })
    .click();
  await expect(
    friend.locator('[data-source="screen_share"] video'),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Parar compartilhamento', exact: true })
    .click();
  await expect(
    friend.locator('[data-source="screen_share"] video'),
  ).toHaveCount(0);
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
