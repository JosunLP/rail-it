// Antworten der API von railitd (siehe docs/api.md). Nur die Felder, die der Bedienplatz
// verwendet.

export type Pos = [number, number];

export interface Layout {
  network: string;
  width: number;
  height: number;
  tracks: { name: string; tvd: string | null; area: string | null; line: Pos[]; slip?: string | null }[];
  points: {
    name: string;
    tvd: string | null;
    area: string | null;
    tip: Pos;
    fork: Pos;
    left: Pos;
    right: Pos;
    /** Kreuzungsweiche, zu der das Zungenpaar gehört. */
    slip?: string | null;
  }[];
  terminals: { name: string; kind: 'boundary' | 'buffer_stop' | 'border'; at: Pos; opens: 'left' | 'right' }[];
  signals: { name: string; kind: string; system: string; at: Pos; facing: 'left' | 'right'; track: string }[];
  level_crossings: { name: string; at: Pos }[];
  crossings?: { name: string; at: [Pos, Pos] }[];
  borders: { left: string; right: string; at: Pos }[];
  areas: { name: string; x0: number; x1: number }[];
  stations: { name: string; at: Pos }[];
  slips?: { name: string; at: Pos }[];
}

export type Occupancy = 'clear' | 'occupied' | 'disturbed';
export type RoutePhase =
  | 'idle'
  | 'setting'
  | 'locked'
  | 'proceed'
  | 'occupied'
  | 'arrived'
  | 'cancelling'
  | 'halted';

export interface NamedAux {
  kind: AuxKind;
  element: string;
  position?: 'left' | 'right';
}

export type AuxKind = 'release_route' | 'reset_tvd' | 'substitute' | 'force_point' | 'take_permission';

export interface PendingAux {
  area: string;
  aux: number;
  op: NamedAux;
  operator: number;
  since_ms: number;
  check: string;
}

/** Zug der Betriebsführung. */
export interface TrainState {
  number: string;
  berth: string | null;
  left: boolean;
  timetabled: boolean;
  hold: boolean;
  next_stop: string | null;
  at_stop: string | null;
  delay_s: number | null;
  extra_delay_s: number;
}

/** Betriebsführung (Zugnummern, Zuglenkung). */
export interface OpsState {
  ars: boolean;
  timetable: string;
  trains: TrainState[];
}

/** Anbindung eines Stellbereichs an seinen Knoten des sicheren Rechners. */
export interface NodeState {
  link: string;
  up: boolean;
  available: boolean;
  synced: boolean;
  cycles: number;
  syncs: number;
  divergences: number;
  link_losses: number;
  protocol_errors: number;
  last_problem: string | null;
}

export interface BorderState {
  name: string;
  holding: boolean;
  requested: boolean;
  offering: boolean;
  approach_locked: boolean;
  peer_holding: boolean | null;
}

export interface Snapshot {
  network: string;
  time_ms: number;
  cycle: number;
  safe_state: string | null;
  areas: {
    name: string;
    safe_state: string | null;
    borders: BorderState[];
    safety: {
      redundancy: string;
      channels: { channel: number; active: boolean }[];
      trip: unknown;
      node?: NodeState | null;
    };
  }[];
  points: {
    area: string;
    name: string;
    report: string;
    commanded: string | null;
    locked_by: string | null;
    holds: number;
  }[];
  signals: {
    area: string;
    name: string;
    commanded: 'stop' | 'proceed';
    report: string;
    substitute: boolean;
    picture: string;
  }[];
  tvds: { area: string; name: string; occupancy: Occupancy; locked_by: string | null; overlap_of: string | null }[];
  routes: { area: string; name: string; start: string; end: string; phase: RoutePhase }[];
  level_crossings: { name: string; report: string; close_commanded: boolean }[];
  pending_aux: PendingAux[];
  aux_counter: number;
  ops?: OpsState | null;
}

/// Antwort auf eine Anforderung (`accepted` oder `rejected`).
export interface Answer {
  type: 'accepted' | 'rejected';
  reason?: string;
  element?: string | null;
  area?: string;
  aux?: { type: string; aux?: number; counter?: number };
}

/// Journaldatei eines Stellbereichs (`/v1/journal`).
export interface JournalInfo {
  area: string;
  file: string;
  bytes: number;
}

/// Zyklus mit Befehlen oder Ereignissen aus der Rückschau (`/v1/journal/events`).
export interface ReviewCycle {
  cycle: number;
  now_ms: number;
  commands: Record<string, unknown>[];
  events: Record<string, unknown>[];
}

/// Zeile der Rückschau: Zeitpunkt, Bereich, Text.
export interface ReviewRow {
  now_ms: number;
  area: string;
  kind: 'command' | 'event';
  text: string;
}
