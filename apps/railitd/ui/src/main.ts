// Bedienplatz von rail-it (Konzept, Abschnitt 14.3).
//
// - Spurplan mit Zuständen aus dem Zustandsbild (`/v1/state`, alle 300 ms).
// - Anzeigeüberwachung: Kommt kein Zustandsbild mit fortgeschrittener Zeit, gilt die Anzeige
//   nach 2 s als gestört; der Spurplan wird abgedeckt, Bedienungen sind gesperrt (REQ-UI-002).
// - Bedienung: Fahrstraßen per Start- und Zielsignal, Kontextmenü für Rücknahme, Weichen,
//   Erlaubnisse und Hilfshandlungen.
// - Hilfshandlungen: Bestätigung nur über die sichere Anzeige- und Bestätigungskette – der
//   zweite Bediener gibt das Element selbst ein, der Bedienplatz berechnet den Prüfcode aus
//   seiner Anzeige (REQ-UI-003).
// - Rückschau: Spurplan, Befehle und Ereignisse zu einem vergangenen Zeitpunkt aus dem Journal
//   des Dienstes; deutlich gekennzeichnet, ohne Bedienung (REQ-UI-005).

import { $ } from '@bquery/bquery/core';
import { effect, signal, usePolling } from '@bquery/bquery/reactive';
import * as api from './api';
import { auxCheck } from './check';
import {
  activeRoutesFrom,
  auxText,
  clock,
  COMMAND_HELP,
  findRoute,
  health,
  mergeSnapshots,
  operable,
  parseCommand,
  reasonText,
  reviewRows,
  routesFrom,
} from './model';
import { displayHealth, messages, report, reviewAt, reviewRowsSignal, snapshot, standbyRole } from './panels';
import { type PlanTarget, PlanView } from './plan';
import type { Answer, AuxKind, JournalInfo, NamedAux, PendingAux, Snapshot } from './types';

const POLL_MS = 300;
/** Die Rückschau zeigt Befehle und Ereignisse dieser Spanne vor dem gewählten Zeitpunkt. */
const REVIEW_SPAN_MS = 60_000;

/** Zeitpunkt (Uhr des Bedienplatzes), zu dem zuletzt die Zeit des Stellwerks fortschritt. */
let lastAdvance = 0;
let lastTime = -1;
const now = signal(Date.now());
setInterval(() => {
  now.value = Date.now();
}, 250);

effect(() => {
  displayHealth.value = health(snapshot.value, lastAdvance, now.value);
});

effect(() => {
  const h = displayHealth.value;
  const reviewing = reviewAt.value !== undefined || standbyRole.value;
  $('body').toggleClass('stale', h === 'stale').toggleClass('safe-state', h === 'safe_state');
  for (const b of Array.from(document.querySelectorAll('[data-command]'))) {
    (b as HTMLButtonElement).disabled = !operable(h) || reviewing;
  }
});

function guard(): boolean {
  if (standbyRole.peek()) {
    report('warn', 'Bereitschaftsrechner: keine Bedienung');
    return false;
  }
  if (reviewAt.peek() !== undefined) {
    report('warn', 'Rückschau: keine Bedienung – erst zurück zum Betrieb');
    return false;
  }
  if (!operable(displayHealth.peek())) {
    report('error', 'Anzeige gestört: keine Bedienung möglich');
    return false;
  }
  return true;
}

/** Antwort des Kerns melden. */
function answered(what: string, a: Answer): boolean {
  if (a.type === 'accepted') {
    const extra =
      a.aux?.type === 'aux_pending'
        ? ` – Hilfshandlung #${a.aux.aux} wartet auf Bestätigung`
        : a.aux?.type === 'aux_executed'
          ? ` – ausgeführt, Zählwerk ${a.aux.counter}`
          : '';
    report('info', `${what}: angenommen${extra}`);
    return true;
  }
  report('warn', `${what}: abgelehnt – ${reasonText(a.reason, a.element)}`);
  return false;
}

async function act(what: string, run: () => Promise<Answer>): Promise<boolean> {
  if (!guard()) return false;
  try {
    return answered(what, await run());
  } catch (e) {
    report('error', `${what}: ${(e as Error).message}`);
    return false;
  }
}

// ── Kontextmenü ─────────────────────────────────────────────────────────────────────────

interface MenuItem {
  label: string;
  run: () => void;
}

function showMenu(x: number, y: number, title: string, items: MenuItem[]): void {
  const menu = document.getElementById('menu');
  if (!menu || items.length === 0) return;
  const head = document.createElement('div');
  head.className = 'menu-title';
  head.textContent = title;
  const buttons = items.map((item) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = item.label;
    b.setAttribute('data-command', '');
    b.disabled = !operable(displayHealth.peek()) || reviewAt.peek() !== undefined || standbyRole.peek();
    $(b).on('click', () => {
      hideMenu();
      item.run();
    });
    return b;
  });
  menu.replaceChildren(head, ...buttons);
  $(menu).css({ left: `${x}px`, top: `${y}px` }).show('flex');
}

function hideMenu(): void {
  const menu = document.getElementById('menu');
  if (menu) $(menu).hide();
}

function auxItem(label: string, op: NamedAux): MenuItem {
  return { label: `${label} (Hilfshandlung)`, run: () => void act(auxText(op), () => api.requestAux(op)) };
}

function menuFor(target: PlanTarget, s: Snapshot): { title: string; items: MenuItem[] } {
  switch (target.kind) {
    case 'signal': {
      const items: MenuItem[] = activeRoutesFrom(s, target.name).flatMap((r) => [
        { label: `Fahrstraße ${r} zurücknehmen`, run: () => void act(`Rücknahme ${r}`, () => api.cancelRoute(r)) },
        auxItem(`Hilfsauflösung ${r}`, { kind: 'release_route', element: r }),
      ]);
      for (const r of routesFrom(s, target.name)) {
        items.push(auxItem(`Ersatzsignal für ${r}`, { kind: 'substitute', element: r }));
      }
      return { title: `Signal ${target.name}`, items };
    }
    case 'point':
      return {
        title: `Weiche ${target.name}`,
        items: (['left', 'right'] as const).flatMap((position) => [
          {
            label: `nach ${position === 'left' ? 'links' : 'rechts'} umstellen`,
            run: () => void act(`Weiche ${target.name}`, () => api.movePoint(target.name, position)),
          },
          auxItem(`hilfsweise nach ${position === 'left' ? 'links' : 'rechts'}`, {
            kind: 'force_point',
            element: target.name,
            position,
          }),
        ]),
      };
    case 'track':
      return {
        title: `Abschnitt ${target.tvd}`,
        items: [auxItem('Achszählgrundstellung', { kind: 'reset_tvd', element: target.tvd })],
      };
    case 'border': {
      const items: MenuItem[] = [
        [target.left, target.right],
        [target.right, target.left],
      ].map(([area, peer]) => ({
        label: `Erlaubnis für ${area} anfordern`,
        run: () => void act(`Erlaubnis ${area}>${peer}`, () => api.requestPermission(`${area}>${peer}`)),
      }));
      items.push(
        auxItem(`Erlaubnis für ${target.left} zurückholen`, {
          kind: 'take_permission',
          element: `${target.left}>${target.right}`,
        }),
      );
      return { title: `Grenze ${target.left} | ${target.right}`, items };
    }
    case 'empty':
      return { title: '', items: [] };
  }
}

// ── Fahrstraßen per Start- und Zielsignal ───────────────────────────────────────────────

let start: string | undefined;

function select(plan: PlanView, target: PlanTarget): void {
  hideMenu();
  const s = snapshot.peek();
  if (target.kind !== 'signal' || !s) {
    start = undefined;
    plan.select(undefined);
    return;
  }
  if (!start) {
    if (routesFrom(s, target.name).length === 0) {
      report('info', `Signal ${target.name}: keine Fahrstraße beginnt hier`);
      return;
    }
    start = target.name;
    plan.select(start);
    report('info', `Start ${start} gewählt – Zielsignal anklicken`);
    return;
  }
  const route = findRoute(s, start, target.name);
  const from = start;
  start = undefined;
  plan.select(undefined);
  if (!route) {
    report('warn', `keine Fahrstraße von ${from} nach ${target.name}`);
    return;
  }
  void act(`Fahrstraße ${route}`, () => api.setRoute(route));
}

// ── Bestätigung einer Hilfshandlung (sichere Anzeige- und Bestätigungskette) ─────────────

async function openConfirm(p: PendingAux): Promise<void> {
  const dialog = document.getElementById('confirm') as HTMLDialogElement | null;
  if (!dialog || !guard()) return;
  // Was der Bedienplatz anzeigt, bildet den Prüfcode.
  const shown: NamedAux = { kind: p.op.kind, element: p.op.element, position: p.op.position };
  const code = await auxCheck(p.area, p.aux, shown);
  const display = code === p.check;
  $('#confirm-text').text(`#${p.aux} ${auxText(shown)}`);
  $('#confirm-meta').text(`Stellbereich ${p.area} · angefordert von Bediener ${p.operator}`);
  $('#confirm-code').text(display ? code : `${code} ≠ ${p.check}`);
  $('#confirm-warning').text(
    display ? '' : 'Anzeige gestört: Prüfcode weicht vom Stellwerk ab. Keine Bestätigung möglich.',
  );
  const input = document.getElementById('confirm-element') as HTMLInputElement;
  input.value = '';
  const ok = document.getElementById('confirm-ok') as HTMLButtonElement;
  ok.disabled = !display;
  dialog.dataset.aux = String(p.aux);
  dialog.dataset.area = p.area;
  dialog.showModal();
  input.focus();
}

async function submitConfirm(): Promise<void> {
  const dialog = document.getElementById('confirm') as HTMLDialogElement;
  const p = snapshot
    .peek()
    ?.pending_aux.find((x) => String(x.aux) === dialog.dataset.aux && x.area === dialog.dataset.area);
  dialog.close();
  if (!p) {
    report('warn', 'Hilfshandlung nicht mehr offen');
    return;
  }
  const typed = ($('#confirm-element').val() as string).trim();
  // Der zweite Bediener nennt das Element selbst; Art und Lage stammen aus der Anzeige.
  const restated: NamedAux = { kind: p.op.kind as AuxKind, element: typed, position: p.op.position };
  const check = await auxCheck(p.area, p.aux, { kind: p.op.kind, element: p.op.element, position: p.op.position });
  await act(`Bestätigung #${p.aux}`, () => api.confirmAux(p, restated, check));
}

// ── Betriebsführung und Stelltisch-Eingabe ──────────────────────────────────────────────

async function ops(what: string, run: () => Promise<string | undefined>): Promise<void> {
  if (!guard()) return;
  try {
    const error = await run();
    report(error ? 'warn' : 'info', error ? `${what}: ${error}` : `${what}: ausgeführt`);
  } catch (e) {
    report('error', `${what}: ${(e as Error).message}`);
  }
}

function command(text: string): void {
  const s = snapshot.peek();
  if (!s) return;
  const c = parseCommand(text, s);
  switch (c.kind) {
    case 'set_route':
      void act(`Fahrstraße ${c.route}`, () => api.setRoute(c.route));
      break;
    case 'cancel_route':
      void act(`Rücknahme ${c.route}`, () => api.cancelRoute(c.route));
      break;
    case 'move_point':
      void act(`Weiche ${c.point}`, () => api.movePoint(c.point, c.position));
      break;
    case 'error':
      report('warn', c.message);
      break;
  }
}

// ── Rückschau (REQ-UI-005) ─────────────────────────────────────────────────────────────

/** Zustandsbild der Rückschau (alle Bereiche zusammengeführt). */
const reviewSnapshot = signal<Snapshot | undefined>(undefined);
let journals: JournalInfo[] = [];
/** Laufende Nummer der Abfrage: Nur die jüngste wird angezeigt. */
let reviewSeq = 0;

async function loadReview(at: number): Promise<void> {
  reviewSeq += 1;
  const seq = reviewSeq;
  try {
    const [states, events] = await Promise.all([
      Promise.all(journals.map((j) => api.getJournalState(at, j.area))),
      Promise.all(
        journals.map(async (j) => ({
          area: j.area,
          cycles: await api.getJournalEvents(at - REVIEW_SPAN_MS, at, j.area),
        })),
      ),
    ]);
    if (seq !== reviewSeq || reviewAt.peek() === undefined) return;
    reviewSnapshot.value = mergeSnapshots(states);
    reviewRowsSignal.value = reviewRows(events);
    $('#review-plan-time').text(clock(at));
  } catch (e) {
    report('error', `Rückschau: ${(e as Error).message}`);
  }
}

function slider(): HTMLInputElement {
  return document.getElementById('review-slider') as HTMLInputElement;
}

function setReview(at: number | undefined): void {
  reviewAt.value = at;
  $('body').toggleClass('review', at !== undefined);
  if (at === undefined) {
    reviewSeq += 1;
    reviewSnapshot.value = undefined;
    reviewRowsSignal.value = [];
    return;
  }
  slider().value = String(at);
  $('#review-time').text(clock(at));
  void loadReview(at);
}

function setupReview(): void {
  const toggle = document.getElementById('review-toggle') as HTMLButtonElement | null;
  if (!toggle) return;
  if (journals.length === 0) {
    toggle.disabled = true;
    toggle.title = 'Rückschau braucht ein Journal (railitd --journal)';
  }
  $(toggle).on('click', () => {
    if (reviewAt.peek() !== undefined) {
      setReview(undefined);
      return;
    }
    const live = snapshot.peek()?.time_ms ?? 0;
    slider().max = String(live);
    setReview(live);
  });
  let pending: ReturnType<typeof setTimeout> | undefined;
  $(slider()).on('input', () => {
    const at = Number(slider().value);
    $('#review-time').text(clock(at));
    reviewAt.value = at;
    clearTimeout(pending);
    pending = setTimeout(() => void loadReview(at), 200);
  });
  const step = (delta: number) => {
    const at = Math.min(Number(slider().max), Math.max(0, (reviewAt.peek() ?? 0) + delta));
    setReview(at);
  };
  $('#review-back').on('click', () => step(-10_000));
  $('#review-forward').on('click', () => step(10_000));
  $('#review-exit').on('click', () => setReview(undefined));
}

// ── Anmeldung und Start ────────────────────────────────────────────────────────────────

async function login(message: string): Promise<void> {
  const dialog = document.getElementById('login') as HTMLDialogElement;
  $('#login-message').text(message);
  dialog.showModal();
  await new Promise<void>((resolve) => {
    $('#login-form').once('submit', () => resolve());
  });
  api.setToken(($('#login-token').val() as string) ?? '');
}

async function main(): Promise<void> {
  $('#operator').val(api.operator.peek());
  $('#operator').on('change', (e) => api.setOperator((e.target as HTMLInputElement).value));

  let layout;
  for (;;) {
    try {
      layout = await api.getLayout();
      break;
    } catch (e) {
      const err = e as api.ApiError;
      await login(err.status === 401 ? 'Bitte mit Zugangstoken anmelden.' : err.message);
    }
  }
  document.title = `rail-it · ${layout.network}`;
  const plan = new PlanView(document.getElementById('plan') as Element, layout, {
    select: (t) => select(plan, t),
    menu: (t, x, y) => {
      const s = snapshot.peek();
      if (!s) return;
      const { title, items } = menuFor(t, s);
      showMenu(x, y, title, items);
    },
  });

  const poll = usePolling<Snapshot>('/v1/state', {
    interval: POLL_MS,
    timeout: 1500,
    fetcher: api.authFetch as typeof fetch,
    pauseOnHidden: false,
    onSuccess: (s) => {
      if (s.time_ms !== lastTime) {
        lastTime = s.time_ms;
        lastAdvance = Date.now();
      }
      snapshot.value = s;
    },
  });
  void poll;

  // In der Rückschau zeigt der Spurplan den Zustand aus dem Journal.
  effect(() => {
    const s = reviewAt.value !== undefined ? reviewSnapshot.value : snapshot.value;
    if (s) plan.update(s);
  });
  const role = await api.getRole().catch(() => ({ role: 'primary' as const, primary: undefined }));
  if (role.role === 'standby') {
    standbyRole.value = true;
    $('body').addClass('standby');
    $('#standby-primary').text(role.primary ?? '');
    report('info', `Bereitschaftsrechner: liest das Journal von ${role.primary ?? '?'} mit`);
  }
  journals = await api.getJournals().catch(() => []);
  setupReview();

  $('#aux').delegate('click', 'button.confirm', (_e, target) => {
    const s = snapshot.peek();
    const p = s?.pending_aux.find(
      (x) => String(x.aux) === target.getAttribute('data-aux') && x.area === target.getAttribute('data-area'),
    );
    if (p) void openConfirm(p);
  });
  $('#confirm-form').on('submit', (e) => {
    e.preventDefault();
    void submitConfirm();
  });
  $('#confirm-cancel').on('click', () => (document.getElementById('confirm') as HTMLDialogElement).close());
  $('#trains').delegate('click', 'button[data-ars]', (_e, target) => {
    const on = target.getAttribute('data-ars') === 'true';
    void ops(on ? 'Zuglenkung ein' : 'Zuglenkung aus', () => api.setArs(on));
  });
  $('#trains').delegate('click', 'button[data-hold]', (_e, target) => {
    const train = target.getAttribute('data-hold') ?? '';
    const hold = target.getAttribute('data-on') === 'true';
    void ops(`${train} ${hold ? 'zurückhalten' : 'freigeben'}`, () => api.holdTrain(train, hold));
  });
  const input = document.getElementById('command') as HTMLInputElement | null;
  if (input) input.placeholder = COMMAND_HELP;
  $('#command-form').on('submit', (e) => {
    e.preventDefault();
    const text = ($('#command').val() as string) ?? '';
    $('#command').val('');
    if (text.trim()) command(text);
  });
  $(document.body).on('click', (e) => {
    if (!(e.target as Element).closest('#menu, svg')) hideMenu();
  });
  $(document.body).on('keydown', (e) => {
    if ((e as KeyboardEvent).key === 'Escape') {
      hideMenu();
      start = undefined;
      plan.select(undefined);
    }
  });

  api.streamEvents((event) => {
    const type = String(event.type ?? '');
    const area = event.area ? `${String(event.area)}: ` : '';
    if (type === 'alarm') report('error', `${area}Alarm ${String(event.alarm)} ${String(event.element ?? '')}`);
    if (type === 'safety') report('error', `${area}Sicherheitsschicht: ${String(event.event)}`);
    if (type === 'aux_executed') report('warn', `${area}Hilfshandlung #${String(event.aux)} ausgeführt`);
    if (type === 'aux_expired') report('warn', `${area}Hilfshandlung #${String(event.aux)} verfallen`);
  });
  report('info', `Bedienplatz verbunden: ${layout.network}`);
  void messages;
}

void main();
