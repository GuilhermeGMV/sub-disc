import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';

test('desktop Rust e navegador decodificam áudio e vídeo nos dois sentidos', async ({
  page,
  playwright,
}) => {
  const binary = process.env.SUBDISC_NATIVE_BINARY;
  test.skip(
    !binary,
    'Defina SUBDISC_NATIVE_BINARY para executar a interoperabilidade real com o desktop',
  );
  const seconds = Number(process.env.SUBDISC_INTEROP_SECONDS ?? 25);
  test.setTimeout((seconds + 60) * 1000);
  const headers = { Origin: 'http://127.0.0.1:18080' };
  const setup = await (await page.request.get('/api/setup')).json();
  if (setup.needs_setup)
    await page.request.post('/api/setup', {
      headers,
      data: {
        username: 'admin',
        password: 'strong password 123',
        setup_key: 'a-safe-setup-test-key',
      },
    });
  else
    await page.request.post('/api/login', {
      headers,
      data: { username: 'admin', password: 'strong password 123' },
    });
  const invite = await (
    await page.request.post('/api/invites', { headers, data: { hours: 1 } })
  ).json();
  const suffix = String(Date.now());
  const nativeName = `native${suffix}`;
  const native = await playwright.request.newContext({
    baseURL: 'http://127.0.0.1:18080',
  });
  expect(
    (
      await native.post('/api/register', {
        headers,
        data: {
          username: nativeName,
          password: 'strong password 123',
          invite: invite.token,
        },
      })
    ).ok(),
  ).toBeTruthy();
  await native.dispose();
  const channel = await (
    await page.request.post('/api/channels', {
      headers,
      data: { name: `interop-${suffix}`, kind: 'voice' },
    })
  ).json();
  const webInvite = await (
    await page.request.post('/api/invites', { headers, data: { hours: 1 } })
  ).json();
  expect(
    (
      await page.request.post('/api/register', {
        headers,
        data: {
          username: `web${suffix}`,
          password: 'strong password 123',
          invite: webInvite.token,
        },
      })
    ).ok(),
  ).toBeTruthy();
  const child = spawn(binary!, ['--media-probe'], {
    env: {
      ...process.env,
      SUBDISC_PROBE_SERVER: 'http://127.0.0.1:18080',
      SUBDISC_PROBE_CHANNEL: channel.id,
      SUBDISC_PROBE_USER: nativeName,
      SUBDISC_PROBE_PASSWORD: 'strong password 123',
      SUBDISC_PROBE_SECONDS: String(seconds),
    },
  });
  let output = '';
  let errors = '';
  let exit: number | null | undefined;
  child.stdout.on('data', (data) => (output += data.toString()));
  child.stderr.on('data', (data) => (errors += data.toString()));
  child.on('close', (code) => (exit = code));
  const done = new Promise<void>((resolve) =>
    child.on('close', () => resolve()),
  );
  try {
    await expect
      .poll(() => ({
        ready: output.includes('"event":"ready"'),
        errors: exit === undefined ? '' : errors,
      }))
      .toMatchObject({ ready: true });
    await page.goto('/');
    await page
      .getByRole('button', { name: new RegExp('^' + channel.name) })
      .click();
    await expect(page.getByTestId('voice-status')).toHaveText(
      'Chamada conectada',
    );
    await expect(page.getByTestId('participant')).toHaveCount(2);
    await page
      .getByRole('button', { name: 'Ativar câmera', exact: true })
      .click();
    const video = page.locator('[data-source="screen_share"] video');
    await expect(video).toBeVisible();
    await expect
      .poll(() =>
        video.evaluate(
          (e: HTMLVideoElement) => e.getVideoPlaybackQuality().totalVideoFrames,
        ),
      )
      .toBeGreaterThan(5);
    await expect
      .poll(() => output.match(/"audio_buffers":([1-9][0-9]*)/)?.[1])
      .toBeTruthy();
    await expect
      .poll(() => output.match(/"video_frames":([1-9][0-9]*)/)?.[1])
      .toBeTruthy();
    if (process.env.SUBDISC_PROBE_REPUBLISH_AT) {
      const trackId = await video.evaluate(
        (e: HTMLVideoElement) =>
          (e.srcObject as MediaStream).getVideoTracks()[0].id,
      );
      await expect
        .poll(() => output.includes('"event":"republished"'), {
          timeout: (seconds + 10) * 1000,
        })
        .toBe(true);
      await expect
        .poll(() =>
          video.evaluate(
            (e: HTMLVideoElement) =>
              (e.srcObject as MediaStream).getVideoTracks()[0].id,
          ),
        )
        .not.toBe(trackId);
      await expect
        .poll(() =>
          video.evaluate(
            (e: HTMLVideoElement) =>
              e.getVideoPlaybackQuality().totalVideoFrames,
          ),
        )
        .toBeGreaterThan(5);
    }
    await done;
    expect(exit, errors + '\n' + output).toBe(0);
    expect(output).toContain('"event":"complete"');
    await page.getByRole('button', { name: 'Sair da chamada' }).click();
    console.log('Native interoperability:', output.trim());
  } finally {
    if (exit !== 0) console.log('Native logs:', output, errors);
    if (exit === undefined) {
      child.kill('SIGTERM');
      await done;
    }
  }
});
