// Anzeigemodell: reine Funktionen von Zustandsbild und Spurplan zu Darstellung und
// Bedienhandlungen. Ohne DOM, damit sie sich mit `bun test` prüfen lassen.

import type { AuxKind, NamedAux, Snapshot } from './types';

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
