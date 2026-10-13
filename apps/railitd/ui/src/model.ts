// Anzeigemodell: reine Funktionen von Zustandsbild und Spurplan zu Darstellung und
// Bedienhandlungen. Ohne DOM, damit sie sich mit `bun test` prüfen lassen.

import type { AuxKind, NamedAux, ReviewCycle, ReviewRow, Snapshot, TrainState } from './types';

/** Nach so vielen Millisekunden ohne neues Zustandsbild gilt die Anzeige als gestört. */
export const STALE_AFTER_MS = 2000;

/** Zustand der Anzeige (REQ-UI-002). */
export type DisplayHealth = 'ok' | 'stale' | 'safe_state';

/**
 * Stellbereich, dessen Knoten des sicheren Rechners getrennt ist oder dessen Spiegel noch
 * nicht abgeglichen ist: Sein Zustandsbild ist nicht verlässlich.
 */
export function nodeProblem(snapshot: Snapshot): string | undefined {
  for (const a of snapshot.areas) {
    const n = a.safety.node;
    if (!n) continue;
    if (!n.available) return `${a.name}: Knoten des sicheren Rechners getrennt`;
    if (!n.synced) return `${a.name}: Spiegel wird mit dem Knoten abgeglichen`;
  }
  return undefined;
}

/**
 * Ist die Anzeige aktuell? `lastAdvance` ist der Zeitpunkt (Uhr des Bedienplatzes), zu dem
 * zuletzt ein Zustandsbild mit fortgeschrittener Zeit ankam. Ein getrennter oder nicht
 * abgeglichener Knoten macht die Anzeige ebenfalls unzuverlässig.
 */
export function health(snapshot: Snapshot | undefined, lastAdvance: number, now: number): DisplayHealth {
  if (!snapshot || now - lastAdvance > STALE_AFTER_MS || nodeProblem(snapshot)) {
    return 'stale';
  }
  return snapshot.safe_state ? 'safe_state' : 'ok';
}

/** Bedienungen sind nur bei aktueller Anzeige möglich (sicherer Zustand: der Kern lehnt ab). */
export function operable(h: DisplayHealth): boolean {
  return h !== 'stale';
}

/** Fahrstraße von `start` nach `end`, falls es sie gibt. */
export function findRoute(snapshot: Snapshot, start: string, end: string): string | undefined {
  return snapshot.routes.find((r) => r.start === start && r.end === end)?.name;
}

/** Eingestellte Fahrstraßen ab einem Signal. */
export function activeRoutesFrom(snapshot: Snapshot, signal: string): string[] {
  return snapshot.routes.filter((r) => r.start === signal && r.phase !== 'idle').map((r) => r.name);
}

/** Alle Fahrstraßen ab einem Signal. */
export function routesFrom(snapshot: Snapshot, signal: string): string[] {
  return snapshot.routes.filter((r) => r.start === signal).map((r) => r.name);
}

/** CSS-Klasse eines Gleisfreimeldeabschnitts. */
export function tvdClass(snapshot: Snapshot, tvd: string | null): string {
  const t = tvd ? snapshot.tvds.find((x) => x.name === tvd) : undefined;
  if (!t) return 'tvd-unknown';
  if (t.occupancy === 'disturbed') return 'tvd-disturbed';
  if (t.occupancy === 'occupied') return 'tvd-occupied';
  if (t.locked_by) return 'tvd-route';
  if (t.overlap_of) return 'tvd-overlap';
  return 'tvd-clear';
}

/** CSS-Klasse eines Signals. */
export function signalClass(snapshot: Snapshot, name: string): string {
  const s = snapshot.signals.find((x) => x.name === name);
  if (!s) return 'sig-unknown';
  if (s.report === 'defective') return 'sig-defective';
  if (s.substitute) return 'sig-substitute';
  return s.commanded === 'proceed' ? 'sig-proceed' : 'sig-stop';
}

/** Lage einer Weiche, wie gemeldet: `left`, `right` oder `null` (keine Endlage, gestört). */
export function pointPosition(snapshot: Snapshot, name: string): 'left' | 'right' | null {
  const p = snapshot.points.find((x) => x.name === name);
  return p?.report === 'left' || p?.report === 'right' ? p.report : null;
}

const AUX_TEXT: Record<AuxKind, string> = {
  release_route: 'Fahrstraßenhilfsauflösung',
  reset_tvd: 'Achszählgrundstellung',
  substitute: 'Ersatzsignal',
  force_point: 'Weiche hilfsweise umstellen',
  take_permission: 'Erlaubnis zurückholen',
};

/** Hilfshandlung in Worten. */
export function auxText(op: NamedAux): string {
  const position = op.position === 'left' ? ' nach links' : op.position === 'right' ? ' nach rechts' : '';
  return `${AUX_TEXT[op.kind]} ${op.element}${position}`;
}

/** Ablehnungsgrund in Worten (wie `railit`). */
export function reasonText(reason: string | undefined, element?: string | null): string {
  const e = element ?? '?';
  switch (reason) {
    case 'conflict':
      return 'feindliche Fahrstraße eingestellt';
    case 'tvd_not_clear':
      return `Abschnitt ${e} nicht frei`;
    case 'point_locked':
      return `Weiche ${e} verschlossen`;
    case 'point_not_operable':
      return `Weiche ${e} liegt in einem nicht freien Abschnitt`;
    case 'route_active':
      return 'Fahrstraße bereits eingestellt';
    case 'route_not_active':
      return 'Fahrstraße nicht eingestellt';
    case 'train_in_route':
      return 'ein Zug befährt die Fahrstraße';
    case 'safe_state':
      return 'Kern im sicheren Zustand';
    case 'unknown_element':
      return 'unbekanntes Element';
    case 'signal_held':
      return `Signal ${e} wird als Flankenschutz in Halt gehalten`;
    case 'aux_busy':
      return 'zu viele offene Hilfshandlungen';
    case 'aux_unknown':
      return 'Hilfshandlung unbekannt oder bereits erledigt';
    case 'aux_expired':
      return 'Bestätigungsfrist abgelaufen';
    case 'same_operator':
      return 'Bestätigung muss ein anderer Bediener geben';
    case 'aux_mismatch':
      return 'bestätigte Hilfshandlung weicht von der angeforderten ab; sie ist verfallen';
    case 'check_mismatch':
      return 'Prüfcode stimmt nicht mit der offenen Hilfshandlung überein';
    case 'aux_not_applicable':
      return 'Voraussetzungen der Hilfshandlung nicht erfüllt';
    case 'no_permission':
      return `keine Erlaubnis für ${e}`;
    default:
      return reason ?? 'unbekannt';
  }
}

/** Zugnummern je Signal (Zugnummernmeldung). */
export function berths(snapshot: Snapshot): Map<string, string> {
  const out = new Map<string, string>();
  for (const t of snapshot.ops?.trains ?? []) {
    if (t.berth && !t.left) out.set(t.berth, t.number);
  }
  return out;
}

/** Zeit seit Start als `h:mm:ss`. */
export function clock(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return `${h}:${String(m).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
}

/** Zug in Worten: Standort, nächster Halt, Pünktlichkeit. */
export function trainText(t: TrainState): string {
  const parts = [`an ${t.berth ?? '–'}`];
  if (t.at_stop) parts.push(`hält in ${t.at_stop}`);
  else if (t.next_stop) parts.push(`nächster Halt ${t.next_stop}`);
  if (!t.timetabled) parts.push('ohne Fahrplan');
  else if (t.delay_s === null) parts.push('noch nicht gemessen');
  else if (t.delay_s >= 60) parts.push(`+${Math.round(t.delay_s / 60)} min`);
  else if (t.delay_s <= -60) parts.push(`${Math.round(t.delay_s / 60)} min`);
  else parts.push('pünktlich');
  if (t.extra_delay_s > 0) parts.push(`Wartezeit ${Math.round(t.extra_delay_s / 60)} min`);
  return parts.join(' · ');
}

/** Bedienung über die Tastatur (Stelltisch-Eingabe). */
export type Command =
  | { kind: 'set_route'; route: string }
  | { kind: 'cancel_route'; route: string }
  | { kind: 'move_point'; point: string; position: 'left' | 'right' }
  | { kind: 'error'; message: string };

/** Kurzhilfe für die Eingabezeile. */
export const COMMAND_HELP = 'Start Ziel (z. B. „A N1“) · FRT Fahrstraße · Weiche L/R';

/**
 * Liest eine Eingabe: `A N1` oder `A-N1` (Fahrstraße einstellen), `FRT A-N1` (zurücknehmen),
 * `W1 L` bzw. `W1 R` (Weiche umstellen). Groß- und Kleinschreibung der Befehlswörter ist
 * gleich; Elementnamen gelten, wie sie im Netz heißen.
 */
export function parseCommand(text: string, snapshot: Snapshot): Command {
  const words = text.trim().split(/\s+/).filter((w) => w.length > 0);
  const [first, second, third] = words;
  if (!first) return { kind: 'error', message: 'Eingabe leer' };
  const routeNamed = (name: string) => snapshot.routes.some((r) => r.name === name);
  if (first.toUpperCase() === 'FRT') {
    if (!second || third) return { kind: 'error', message: 'FRT erwartet genau eine Fahrstraße' };
    return routeNamed(second)
      ? { kind: 'cancel_route', route: second }
      : { kind: 'error', message: `Fahrstraße ${second} gibt es nicht` };
  }
  if (words.length === 1) {
    return routeNamed(first)
      ? { kind: 'set_route', route: first }
      : { kind: 'error', message: `Fahrstraße ${first} gibt es nicht` };
  }
  if (words.length === 2 && second) {
    const position = { L: 'left', R: 'right' }[second.toUpperCase()] as 'left' | 'right' | undefined;
    if (position && snapshot.points.some((p) => p.name === first)) {
      return { kind: 'move_point', point: first, position };
    }
    const route = findRoute(snapshot, first, second);
    return route
      ? { kind: 'set_route', route }
      : { kind: 'error', message: `keine Fahrstraße von ${first} nach ${second}` };
  }
  return { kind: 'error', message: `unbekannte Eingabe: ${COMMAND_HELP}` };
}

// ── Rückschau (REQ-UI-005) ──────────────────────────────────────────────────────────────

/** Zustandsbilder je Stellbereich (aus den Journalen) zu einem Bild zusammenführen. */
export function mergeSnapshots(parts: Snapshot[]): Snapshot | undefined {
  const [first, ...rest] = parts;
  if (!first) return undefined;
  const merged: Snapshot = {
    ...first,
    areas: [...first.areas],
    points: [...first.points],
    signals: [...first.signals],
    tvds: [...first.tvds],
    routes: [...first.routes],
    level_crossings: [...first.level_crossings],
    pending_aux: [...first.pending_aux],
    ops: null,
  };
  for (const p of rest) {
    merged.time_ms = Math.max(merged.time_ms, p.time_ms);
    merged.safe_state = merged.safe_state ?? p.safe_state;
    merged.areas.push(...p.areas);
    merged.points.push(...p.points);
    merged.signals.push(...p.signals);
    merged.tvds.push(...p.tvds);
    merged.routes.push(...p.routes);
    merged.level_crossings.push(...p.level_crossings);
    merged.pending_aux.push(...p.pending_aux);
    merged.aux_counter += p.aux_counter;
  }
  return merged;
}

const PHASE_TEXT: Record<string, string> = {
  idle: 'aufgelöst',
  setting: 'wird eingestellt',
  locked: 'festgelegt',
  proceed: 'Fahrt',
  occupied: 'befahren',
  arrived: 'angekommen',
  cancelling: 'wird zurückgenommen',
  halted: 'haltgefallen',
};

/** Stellbefehl aus der Rückschau in Worten. */
export function commandText(c: Record<string, unknown>): string {
  const s = (k: string) => String(c[k] ?? '');
  switch (c.type) {
    case 'move_point':
      return `Weiche ${s('point')} nach ${c.position === 'left' ? 'links' : 'rechts'}`;
    case 'set_signal':
      return `Signal ${s('signal')} auf ${c.aspect === 'proceed' ? 'Fahrt' : 'Halt'}`;
    case 'set_substitute':
      return `Ersatzsignal ${s('signal')} ${c.on ? 'ein' : 'aus'}`;
    case 'set_level_crossing':
      return `Bahnübergang ${s('lx')} ${c.close ? 'schließen' : 'öffnen'}`;
    case 'reset_tvd':
      return `Grundstellung ${s('tvd')}`;
    case 'border':
      return `Grenze ${s('border')}`;
    default:
      return s('type');
  }
}

/** Ereignis aus der Rückschau in Worten. */
export function eventText(e: Record<string, unknown>): string {
  const s = (k: string) => String(e[k] ?? '');
  const at = e.element ? ` (${s('element')})` : '';
  switch (e.type) {
    case 'accepted':
      return `Anforderung ${s('request')} angenommen`;
    case 'rejected':
      return `Anforderung ${s('request')} abgelehnt: ${reasonText(s('reason'), e.element as string | null)}`;
    case 'route_phase':
      return `Fahrstraße ${s('route')}: ${PHASE_TEXT[s('phase')] ?? s('phase')}`;
    case 'alarm':
      return `Meldung ${s('alarm')}${at}`;
    case 'aux_pending':
      return `Hilfshandlung #${s('aux')} angefordert`;
    case 'aux_executed':
      return `Hilfshandlung #${s('aux')} ausgeführt, Zählwerk ${s('counter')}`;
    case 'aux_expired':
      return `Hilfshandlung #${s('aux')} verfallen`;
    case 'permission':
      return `Erlaubnis an ${s('border')} ${e.holding ? 'übernommen' : 'abgegeben'}`;
    default:
      return s('type');
  }
}

/** Zeilen der Rückschau aus den Zyklen aller Bereiche, neueste zuerst. */
export function reviewRows(perArea: { area: string; cycles: ReviewCycle[] }[]): ReviewRow[] {
  const rows: ReviewRow[] = [];
  for (const { area, cycles } of perArea) {
    for (const c of cycles) {
      for (const e of c.events) rows.push({ now_ms: c.now_ms, area, kind: 'event', text: eventText(e) });
      for (const x of c.commands) rows.push({ now_ms: c.now_ms, area, kind: 'command', text: commandText(x) });
    }
  }
  return rows.sort((a, b) => b.now_ms - a.now_ms);
}
