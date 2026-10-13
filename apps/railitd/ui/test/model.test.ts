import { describe, expect, test } from 'bun:test';
import {
  activeRoutesFrom,
  auxText,
  berths,
  clock,
  findRoute,
  health,
  nodeProblem,
  operable,
  pointPosition,
  reasonText,
  routesFrom,
  parseCommand,
  STALE_AFTER_MS,
  signalClass,
  trainText,
  tvdClass,
} from '../src/model';
import type { Snapshot, TrainState } from '../src/types';

function train(patch: Partial<TrainState>): TrainState {
  return {
    number: 'RB 1',
    berth: 'S1',
    left: false,
    timetabled: true,
    hold: false,
    next_stop: null,
    at_stop: null,
    delay_s: 0,
    extra_delay_s: 0,
    ...patch,
  };
}

function snapshot(patch: Partial<Snapshot> = {}): Snapshot {
  return {
    network: 'Test',
    time_ms: 1000,
    cycle: 10,
    safe_state: null,
    areas: [],
    points: [
      { area: 'A', name: 'W1', report: 'left', commanded: 'left', locked_by: null, holds: 0 },
      { area: 'A', name: 'W2', report: 'moving', commanded: 'right', locked_by: null, holds: 0 },
    ],
    signals: [
      { area: 'A', name: 'S1', commanded: 'proceed', report: 'proceed', substitute: false, picture: 'Ks1' },
      { area: 'A', name: 'S2', commanded: 'stop', report: 'stop', substitute: false, picture: 'Hp0' },
      { area: 'A', name: 'S3', commanded: 'stop', report: 'stop', substitute: true, picture: 'Zs1' },
      { area: 'A', name: 'S4', commanded: 'proceed', report: 'defective', substitute: false, picture: 'dark' },
    ],
    tvds: [
      { area: 'A', name: 'T1', occupancy: 'clear', locked_by: null, overlap_of: null },
      { area: 'A', name: 'T2', occupancy: 'clear', locked_by: 'S1-S2', overlap_of: null },
      { area: 'A', name: 'T3', occupancy: 'clear', locked_by: null, overlap_of: 'S1-S2' },
      { area: 'A', name: 'T4', occupancy: 'occupied', locked_by: 'S1-S2', overlap_of: null },
      { area: 'A', name: 'T5', occupancy: 'disturbed', locked_by: null, overlap_of: null },
    ],
    routes: [
      { area: 'A', name: 'S1-S2', start: 'S1', end: 'S2', phase: 'proceed' },
      { area: 'A', name: 'S1-S3', start: 'S1', end: 'S3', phase: 'idle' },
    ],
    level_crossings: [],
    pending_aux: [],
    aux_counter: 0,
    ...patch,
  };
}

describe('Anzeigeüberwachung (REQ-UI-002)', () => {
  test('ohne Zustandsbild gestört', () => {
    expect(health(undefined, 0, 0)).toBe('stale');
  });

  test('aktuell, solange die Zeit des Stellwerks fortschreitet', () => {
    const s = snapshot();
    expect(health(s, 10_000, 10_000)).toBe('ok');
    expect(health(s, 10_000, 10_000 + STALE_AFTER_MS)).toBe('ok');
    expect(health(s, 10_000, 10_001 + STALE_AFTER_MS)).toBe('stale');
  });

  test('getrennter oder nicht abgeglichener Knoten: Anzeige gestört', () => {
    const node = {
      link: 'udp://127.0.0.1:7979',
      up: true,
      available: true,
      synced: true,
      cycles: 10,
      syncs: 1,
      divergences: 0,
      link_losses: 0,
      protocol_errors: 0,
      last_problem: null,
    };
    const area = (n: typeof node) => ({
      name: 'Adorf',
      safe_state: null,
      borders: [],
      safety: { redundancy: 'two_out_of_two', channels: [], trip: null, node: n },
    });
    expect(health(snapshot({ areas: [area(node)] }), 0, 0)).toBe('ok');
    expect(nodeProblem(snapshot({ areas: [area(node)] }))).toBeUndefined();
    const down = snapshot({ areas: [area({ ...node, available: false })] });
    expect(health(down, 0, 0)).toBe('stale');
    expect(nodeProblem(down)).toContain('getrennt');
    const syncing = snapshot({ areas: [area({ ...node, synced: false })] });
    expect(health(syncing, 0, 0)).toBe('stale');
    expect(nodeProblem(syncing)).toContain('abgeglichen');
  });

  test('sicherer Zustand wird angezeigt, Bedienung bleibt möglich (der Kern lehnt ab)', () => {
    const s = snapshot({ safe_state: 'safety_layer' });
    expect(health(s, 0, 0)).toBe('safe_state');
    expect(operable('safe_state')).toBe(true);
    expect(operable('ok')).toBe(true);
    expect(operable('stale')).toBe(false);
  });
});

describe('Darstellung', () => {
  test('Gleisfreimeldeabschnitte', () => {
    const s = snapshot();
    expect(tvdClass(s, 'T1')).toBe('tvd-clear');
    expect(tvdClass(s, 'T2')).toBe('tvd-route');
    expect(tvdClass(s, 'T3')).toBe('tvd-overlap');
    // Besetzt geht vor Fahrstraße, gestört vor allem.
    expect(tvdClass(s, 'T4')).toBe('tvd-occupied');
    expect(tvdClass(s, 'T5')).toBe('tvd-disturbed');
    expect(tvdClass(s, 'T9')).toBe('tvd-unknown');
    expect(tvdClass(s, null)).toBe('tvd-unknown');
  });

  test('Signale', () => {
    const s = snapshot();
    expect(signalClass(s, 'S1')).toBe('sig-proceed');
    expect(signalClass(s, 'S2')).toBe('sig-stop');
    expect(signalClass(s, 'S3')).toBe('sig-substitute');
    expect(signalClass(s, 'S4')).toBe('sig-defective');
    expect(signalClass(s, 'S9')).toBe('sig-unknown');
  });

  test('Weichen: nur Endlagen werden als Lage gezeigt', () => {
    const s = snapshot();
    expect(pointPosition(s, 'W1')).toBe('left');
    expect(pointPosition(s, 'W2')).toBeNull();
    expect(pointPosition(s, 'W9')).toBeNull();
  });

  test('Zugnummern je Signal', () => {
    const s = snapshot({
      ops: {
        ars: true,
        timetable: 'Test',
        trains: [
          train({ number: 'RB 1', berth: 'S1' }),
          train({ number: 'RB 2', berth: 'S2', left: true }),
          train({ number: 'RB 3', berth: null }),
        ],
      },
    });
    expect([...berths(s)]).toEqual([['S1', 'RB 1']]);
    expect(berths(snapshot()).size).toBe(0);
  });

  test('Uhr', () => {
    expect(clock(0)).toBe('0:00:00');
    expect(clock(3_723_999)).toBe('1:02:03');
  });
});

describe('Bedienung', () => {
  test('Fahrstraßen nach Start- und Zielsignal', () => {
    const s = snapshot();
    expect(findRoute(s, 'S1', 'S2')).toBe('S1-S2');
    expect(findRoute(s, 'S2', 'S1')).toBeUndefined();
    expect(routesFrom(s, 'S1')).toEqual(['S1-S2', 'S1-S3']);
    expect(activeRoutesFrom(s, 'S1')).toEqual(['S1-S2']);
    expect(routesFrom(s, 'S2')).toEqual([]);
  });

  test('Hilfshandlungen und Ablehnungen in Worten', () => {
    expect(auxText({ kind: 'release_route', element: 'A-N1' })).toBe('Fahrstraßenhilfsauflösung A-N1');
    expect(auxText({ kind: 'force_point', element: 'W1', position: 'right' })).toBe(
      'Weiche hilfsweise umstellen W1 nach rechts',
    );
    expect(reasonText('aux_mismatch')).toContain('weicht von der angeforderten ab');
    expect(reasonText('tvd_not_clear', 'T1')).toBe('Abschnitt T1 nicht frei');
    expect(reasonText('neu')).toBe('neu');
    expect(reasonText(undefined)).toBe('unbekannt');
  });
});

describe('Betriebsführung', () => {
  test('Zug in Worten', () => {
    expect(trainText(train({ next_stop: 'Adorf/1', delay_s: 130 }))).toBe('an S1 · nächster Halt Adorf/1 · +2 min');
    expect(trainText(train({ at_stop: 'Bstadt/2', delay_s: 0 }))).toBe('an S1 · hält in Bstadt/2 · pünktlich');
    expect(trainText(train({ delay_s: null }))).toBe('an S1 · noch nicht gemessen');
    expect(trainText(train({ timetabled: false, berth: null }))).toBe('an – · ohne Fahrplan');
    expect(trainText(train({ delay_s: -120, extra_delay_s: 60 }))).toBe('an S1 · -2 min · Wartezeit 1 min');
  });
});

describe('Stelltisch-Eingabe (REQ-UI-004)', () => {
  const s = snapshot();
  test('Fahrstraße per Start und Ziel oder Name', () => {
    expect(parseCommand('S1 S2', s)).toEqual({ kind: 'set_route', route: 'S1-S2' });
    expect(parseCommand('  S1-S3 ', s)).toEqual({ kind: 'set_route', route: 'S1-S3' });
    expect(parseCommand('S2 S1', s).kind).toBe('error');
  });
  test('Rücknahme und Weichen', () => {
    expect(parseCommand('frt S1-S2', s)).toEqual({ kind: 'cancel_route', route: 'S1-S2' });
    expect(parseCommand('FRT X', s).kind).toBe('error');
    expect(parseCommand('W1 r', s)).toEqual({ kind: 'move_point', point: 'W1', position: 'right' });
    expect(parseCommand('W1 L', s)).toEqual({ kind: 'move_point', point: 'W1', position: 'left' });
  });
  test('Fehler', () => {
    expect(parseCommand('', s).kind).toBe('error');
    expect(parseCommand('a b c d', s).kind).toBe('error');
  });
});
