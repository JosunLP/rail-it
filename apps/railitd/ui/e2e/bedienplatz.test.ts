// Ende-zu-Ende: Bedienplatz im Browser gegen einen echten railitd mit Beispielstadt.
//
// Voraussetzung: `cargo build -p railit -p railitd` (Pfad zu den Programmen über
// `RAILIT_BIN`, Vorgabe `target/debug` im Arbeitsbereich) und `bun run build`.

import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { type Browser, type Page, chromium } from 'playwright';

const repo = join(import.meta.dir, '../../../..');
const bin = process.env.RAILIT_BIN ?? join(repo, 'target/debug');
const network = join(repo, 'examples/beispielstadt.toml');
const port = 17_000 + Math.floor(Math.random() * 2000);
const base = `http://127.0.0.1:${port}`;

let daemon: ReturnType<typeof Bun.spawn> | undefined;
let standby: ReturnType<typeof Bun.spawn> | undefined;
const standbyBase = `http://127.0.0.1:${port + 1}`;
let browser: Browser;

async function healthy(): Promise<boolean> {
  try {
    return (await fetch(`${base}/v1/health`)).ok;
  } catch {
    return false;
  }
}

beforeAll(async () => {
  const dir = mkdtempSync(join(tmpdir(), 'railit-ui-'));
  const plan = join(dir, 'beispielstadt.rplan');
  const compile = Bun.spawnSync([join(bin, 'railit'), 'plan', 'compile', network, '--out', plan]);
  if (compile.exitCode !== 0) throw new Error(`plan compile: ${compile.stderr.toString()}`);
  daemon = Bun.spawn(
    [
      join(bin, 'railitd'),
      ...['--network', network, '--plan', plan, '--allow-unsigned'],
      ...['--journal', join(dir, 'betrieb.journal'), '--listen', `127.0.0.1:${port}`],
    ],
    { stdout: 'ignore', stderr: 'ignore' },
  );
  for (let i = 0; i < 100 && !(await healthy()); i++) await Bun.sleep(100);
  if (!(await healthy())) throw new Error('railitd startet nicht');
  // Bereitschaftsrechner: liest das Journal des Hauptrechners mit (REQ-SVC-011).
  standby = Bun.spawn(
    [
      join(bin, 'railitd'),
      ...['--network', network, '--plan', plan, '--allow-unsigned'],
      ...['--standby', base, '--listen', `127.0.0.1:${port + 1}`],
    ],
    { stdout: 'ignore', stderr: 'ignore' },
  );
  browser = await chromium.launch();
});

afterAll(async () => {
  await browser?.close();
  daemon?.kill();
  standby?.kill();
});

/** Bedienplatz in eigenem Browserkontext (eigene Sitzung) mit Bedienernummer. */
async function desk(operator: number): Promise<Page> {
  const context = await browser.newContext({ viewport: { width: 1400, height: 800 } });
  const page = await context.newPage();
  page.on('pageerror', (e) => {
    throw e;
  });
  await page.goto(`${base}/`);
  await page.locator('[data-health="ok"]').waitFor();
  await page.locator('#operator').fill(String(operator));
  await page.locator('#operator').dispatchEvent('change');
  return page;
}

const signal = (page: Page, name: string) => page.locator(`[data-signal="${name}"]`);
const hit = (page: Page, name: string) => page.locator(`[data-signal="${name}"] .signal-hit`);

/** Hilfshandlung über das Kontextmenü anfordern, nachdem der Bedienplatz keine mehr zeigt. */
async function requestAux(page: Page, name: string, item: string): Promise<void> {
  await page.locator('rit-aux .empty').waitFor();
  await menu(page, name, item);
}

async function menu(page: Page, name: string, item: string): Promise<void> {
  await hit(page, name).click({ button: 'right' });
  await page.locator('#menu button', { hasText: item }).click();
}

/** Fahrstraße per Klick; zuerst muss der Bedienplatz das Startsignal in Halt zeigen. */
async function setRoute(page: Page, start: string, end: string): Promise<void> {
  await signal(page, start).and(page.locator('.sig-stop')).waitFor({ timeout: 10_000 });
  await hit(page, start).click();
  await signal(page, start).and(page.locator('.selected')).waitFor();
  await hit(page, end).click();
  await signal(page, start).and(page.locator('.sig-proceed')).waitFor({ timeout: 15_000 });
}

/** Die einzige offene Hilfshandlung bestätigen. */
async function confirm(page: Page, element: string): Promise<void> {
  await page.locator('rit-aux li').and(page.locator(':only-child')).locator('button.confirm').click();
  await page.locator('#confirm[open]').waitFor();
  await page.locator('#confirm-element').fill(element);
  await page.locator('#confirm-ok').click();
}

describe('Bedienplatz', () => {
  let one: Page;
  let two: Page;

  test('Spurplan wird aus /v1/layout gezeichnet (REQ-UI-001)', async () => {
    one = await desk(1);
    two = await desk(2);
    expect(await one.locator('svg.plan').count()).toBe(1);
    expect(await one.locator('[data-signal]').count()).toBe(8);
    expect(await one.locator('[data-point]').count()).toBe(2);
    expect(await one.locator('svg.plan .track[data-tvd="T1"]').count()).toBeGreaterThan(0);
    await signal(one, 'A').and(one.locator('.sig-stop')).waitFor();
  });

  test('Fahrstraße per Start- und Zielsignal', async () => {
    await setRoute(one, 'A', 'N1');
    await one.locator('svg.plan .track.tvd-route').first().waitFor({ state: 'attached' });
    // Der zweite Bedienplatz zeigt dasselbe Bild.
    await signal(two, 'A').and(two.locator('.sig-proceed')).waitFor();
  });

  test('Hilfsauflösung: Bestätigung mit abweichendem Element verfällt (REQ-UI-003, REQ-IXL-022)', async () => {
    await requestAux(one, 'A', 'Hilfsauflösung A-N1');
    await two.locator('rit-aux button.confirm').waitFor();
    await confirm(two, 'A-N2');
    await two.locator('rit-log li.msg-warn', { hasText: 'weicht von der angeforderten ab' }).waitFor();
    await two.locator('rit-aux .empty').waitFor();
    expect(await signal(two, 'A').getAttribute('class')).toContain('sig-proceed');
    expect(await two.locator('rit-aux .counter').textContent()).toContain('0');
  });

  test('Derselbe Bediener darf nicht bestätigen', async () => {
    await requestAux(one, 'A', 'Hilfsauflösung A-N1');
    await one.locator('rit-aux button.confirm').waitFor();
    await confirm(one, 'A-N1');
    await one.locator('rit-log li.msg-warn', { hasText: 'anderer Bediener' }).waitFor();
  });

  test('Hilfsauflösung: richtige Bestätigung wird ausgeführt', async () => {
    await two.locator('rit-aux button.confirm').waitFor();
    await confirm(two, 'A-N1');
    await two.locator('rit-log li', { hasText: 'Bestätigung' }).first().waitFor();
    await signal(two, 'A').and(two.locator('.sig-stop')).waitFor({ timeout: 10_000 });
    await two.locator('rit-aux .counter', { hasText: 'Zählwerk 1' }).waitFor();
  });

  test('Verfälschte Anzeige: Prüfcode weicht ab, Bestätigung gesperrt (REQ-UI-003)', async () => {
    await setRoute(one, 'A', 'N1');
    await requestAux(one, 'A', 'Hilfsauflösung A-N1');
    // Die Anzeige von Bedienplatz 2 wird auf dem Weg verfälscht: falsches Element.
    await two.route('**/v1/state', async (route) => {
      const response = await route.fetch();
      const body = await response.json();
      for (const p of body.pending_aux) p.op.element = 'A-N2';
      await route.fulfill({ response, json: body });
    });
    await two.locator('rit-aux .aux-text', { hasText: 'A-N2' }).waitFor();
    await two.locator('rit-aux button.confirm').first().click();
    await two.locator('#confirm[open]').waitFor();
    expect(await two.locator('#confirm-warning').textContent()).toContain('Anzeige gestört');
    expect(await two.locator('#confirm-ok').isDisabled()).toBe(true);
    await two.locator('#confirm-cancel').click();
    await two.unroute('**/v1/state');
  });

  test('Rückschau: Spurplan und Ereignisse aus dem Journal, keine Bedienung (REQ-UI-005)', async () => {
    await signal(two, 'A').and(two.locator('.sig-proceed')).waitFor();
    await two.locator('#review-toggle').click();
    await two.locator('body.review').waitFor();
    expect(await two.locator('.review-banner').isVisible()).toBe(true);
    expect(await two.locator('#command').isDisabled()).toBe(true);
    // Felder des laufenden Betriebs sind ausgeblendet.
    expect(await two.locator('rit-aux').isVisible()).toBe(false);
    await two.locator('rit-review li', { hasText: 'Fahrstraße A-N1' }).first().waitFor();
    await two.locator('rit-review li.review-command', { hasText: 'Signal A auf Fahrt' }).first().waitFor();
    // Am Anfang des Journals stehen alle Signale auf Halt.
    await two.locator('#review-slider').fill('0');
    await signal(two, 'A').and(two.locator('.sig-stop')).waitFor();
    await two.locator('#review-time', { hasText: '0:00:00' }).waitFor();
    // Bedienen ist gesperrt, auch über das Kontextmenü.
    await hit(two, 'A').click({ button: 'right' });
    const item = two.locator('#menu button').first();
    if ((await item.count()) > 0) expect(await item.isDisabled()).toBe(true);
    await two.keyboard.press('Escape');
    // Zurück zum Betrieb: wieder der aktuelle Zustand.
    await two.locator('#review-exit').click();
    await two.locator('body:not(.review)').waitFor();
    await signal(two, 'A').and(two.locator('.sig-proceed')).waitFor();
    expect(await two.locator('#command').isDisabled()).toBe(false);
  });

  test('Bereitschaftsrechner zeigt den Zustand des Hauptrechners ohne Bedienung (REQ-SVC-011)', async () => {
    const context = await browser.newContext({ viewport: { width: 1400, height: 800 } });
    const page = await context.newPage();
    page.on('pageerror', (e) => {
      throw e;
    });
    await page.goto(`${standbyBase}/`);
    await page.locator('body.standby').waitFor({ timeout: 10_000 });
    expect(await page.locator('.standby-banner').isVisible()).toBe(true);
    // Am Hauptrechner ist A-N1 eingestellt (voriger Test), der Bereitschaftsrechner zeigt es.
    await signal(page, 'A').and(page.locator('.sig-proceed')).waitFor({ timeout: 10_000 });
    expect(await page.locator('#command').isDisabled()).toBe(true);
    await context.close();
  });

  test('Anzeigeüberwachung: ohne aktuelles Zustandsbild keine Bedienung (REQ-UI-002)', async () => {
    // Bedienplatz 1 zeigt die offene Hilfshandlung aus dem vorigen Test.
    await one.locator('rit-aux button.confirm').waitFor();
    daemon?.kill();
    await one.locator('body.stale').waitFor({ timeout: 5_000 });
    await one.locator('[data-health="stale"]').waitFor();
    expect(await one.locator('.stale-banner').isVisible()).toBe(true);
    expect(await one.locator('rit-aux button.confirm').first().isDisabled()).toBe(true);
    await hit(one, 'A').click({ button: 'right', force: true });
    const buttons = one.locator('#menu button');
    if ((await buttons.count()) > 0) expect(await buttons.first().isDisabled()).toBe(true);
  });
});
