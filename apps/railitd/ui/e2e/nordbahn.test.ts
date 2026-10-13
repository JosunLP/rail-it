// Ende-zu-Ende: Bedienplatz für ein Netz mit drei Stellbereichen, Fahrplan und Zuglenkung
// (Nordbahn) gegen einen echten railitd (REQ-UI-004).

import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { type Browser, type Page, chromium } from 'playwright';

const repo = join(import.meta.dir, '../../../..');
const bin = process.env.RAILIT_BIN ?? join(repo, 'target/debug');
const network = join(repo, 'examples/nordbahn.toml');
const timetable = join(repo, 'examples/nordbahn-fahrplan.toml');
const port = 19_000 + Math.floor(Math.random() * 2000);
const base = `http://127.0.0.1:${port}`;

let daemon: ReturnType<typeof Bun.spawn> | undefined;
let browser: Browser;
let page: Page;

async function healthy(): Promise<boolean> {
  try {
    return (await fetch(`${base}/v1/health`)).ok;
  } catch {
    return false;
  }
}

beforeAll(async () => {
  const plans = mkdtempSync(join(tmpdir(), 'railit-nordbahn-'));
  const compile = Bun.spawnSync([join(bin, 'railit'), 'plan', 'compile', network, '--out', plans]);
  if (compile.exitCode !== 0) throw new Error(`plan compile: ${compile.stderr.toString()}`);
  daemon = Bun.spawn(
    [
      join(bin, 'railitd'),
      ...['--network', network, '--plan', plans, '--allow-unsigned'],
      ...['--timetable', timetable, '--ars', '--speed', '10'],
      ...['--listen', `127.0.0.1:${port}`],
    ],
    { stdout: 'ignore', stderr: 'ignore' },
  );
  for (let i = 0; i < 100 && !(await healthy()); i++) await Bun.sleep(100);
  if (!(await healthy())) throw new Error('railitd startet nicht');
  browser = await chromium.launch();
  page = await (await browser.newContext({ viewport: { width: 1600, height: 800 } })).newPage();
  page.on('pageerror', (e) => {
    throw e;
  });
  await page.goto(`${base}/`);
  await page.locator('[data-health="ok"]').waitFor();
});

afterAll(async () => {
  await browser?.close();
  daemon?.kill();
});

describe('Bedienplatz Nordbahn', () => {
  test('drei Stellbereiche mit Grenzen im Spurplan (REQ-UI-001)', async () => {
    expect(await page.locator('rit-status .badge[title]').count()).toBe(3);
    expect(await page.locator('svg.plan [data-border-left]').count()).toBe(2);
    // An jeder Grenze zeigt ein Pfeil, wer die Erlaubnis hält.
    await page.locator('svg.plan .border-arrow', { hasText: /[▶◀]/ }).first().waitFor();
  });

  test('Züge nach Fahrplan mit Zuglenkung', async () => {
    await page.locator('rit-trains li.train', { hasText: 'RE2' }).waitFor({ timeout: 20_000 });
    await page.locator('rit-trains button.ars.on').waitFor();
    // Zugnummer am Signal im Spurplan.
    await page.locator('svg.plan .berth', { hasText: /RE[12]/ }).first().waitFor({ timeout: 20_000 });
  });

  test('Disposition: Zug zurückhalten und freigeben', async () => {
    const re2 = page.locator('rit-trains li.train', { hasText: 'RE2' });
    await re2.locator('button.hold', { hasText: 'Zurückhalten' }).click();
    await re2.locator('button.hold', { hasText: 'Freigeben' }).waitFor();
    await page.locator('rit-trains li.held', { hasText: 'RE2' }).waitFor();
    await re2.locator('button.hold', { hasText: 'Freigeben' }).click();
    await re2.locator('button.hold', { hasText: 'Zurückhalten' }).waitFor();
  });

  test('Zuglenkung aus- und wieder einschalten', async () => {
    await page.locator('rit-trains button.ars', { hasText: 'Zuglenkung ein' }).click();
    await page.locator('rit-trains button.ars', { hasText: 'Zuglenkung aus' }).waitFor();
    const state = await (await fetch(`${base}/v1/state`)).json();
    expect(state.ops.ars).toBe(false);
    await page.locator('rit-trains button.ars', { hasText: 'Zuglenkung aus' }).click();
    await page.locator('rit-trains button.ars.on').waitFor();
  });

  test('Stelltisch-Eingabe: Fehlermeldung und Weiche', async () => {
    await page.locator('#command').fill('XX YY');
    await page.locator('#command').press('Enter');
    await page.locator('rit-log li.msg-warn', { hasText: 'keine Fahrstraße von XX nach YY' }).waitFor();
    expect(await page.locator('#command').inputValue()).toBe('');
  });
});
