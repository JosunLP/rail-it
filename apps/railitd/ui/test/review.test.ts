// Rückschau im Bedienplatz (REQ-UI-005): reine Funktionen.

import { describe, expect, test } from 'bun:test';
import { commandText, eventText, mergeSnapshots, reviewRows } from '../src/model';
import type { Snapshot } from '../src/types';

function area(name: string, time: number, signal: string, phase: 'idle' | 'proceed'): Snapshot {
  return {
    network: 'Nordbahn',
    time_ms: time,
    cycle: time / 100,
    safe_state: null,
    areas: [{ name, safe_state: null, borders: [], safety: { redundancy: '1', channels: [], trip: null } }],
    points: [],
    signals: [
      { area: name, name: signal, commanded: phase === 'proceed' ? 'proceed' : 'stop', report: 'stop', substitute: false, picture: '' },
    ],
    tvds: [],
    routes: [{ area: name, name: `${signal}-X`, start: signal, end: 'X', phase }],
    level_crossings: [],
    pending_aux: [],
    aux_counter: 1,
    ops: null,
  };
}

describe('Rückschau', () => {
  test('Zustandsbilder der Bereiche werden zusammengeführt', () => {
    const merged = mergeSnapshots([area('Adorf', 5000, 'A', 'proceed'), area('Bdorf', 5100, 'B', 'idle')]);
    expect(merged?.areas.map((a) => a.name)).toEqual(['Adorf', 'Bdorf']);
    expect(merged?.signals.map((s) => s.name)).toEqual(['A', 'B']);
    expect(merged?.routes.length).toBe(2);
    expect(merged?.aux_counter).toBe(2);
    expect(merged?.time_ms).toBe(5100);
    expect(mergeSnapshots([])).toBeUndefined();
  });

  test('Teile bleiben unverändert', () => {
    const a = area('Adorf', 5000, 'A', 'proceed');
    mergeSnapshots([a, area('Bdorf', 5100, 'B', 'idle')]);
    expect(a.signals.length).toBe(1);
  });

  test('Befehle und Ereignisse in Worten', () => {
    expect(commandText({ type: 'move_point', point: 'W1', position: 'left' })).toBe('Weiche W1 nach links');
    expect(commandText({ type: 'set_signal', signal: 'A', aspect: 'proceed' })).toBe('Signal A auf Fahrt');
    expect(eventText({ type: 'route_phase', route: 'A-N1', phase: 'proceed' })).toBe('Fahrstraße A-N1: Fahrt');
    expect(eventText({ type: 'aux_executed', aux: 3, counter: 7 })).toBe('Hilfshandlung #3 ausgeführt, Zählwerk 7');
    expect(eventText({ type: 'alarm', alarm: 'signal_dropped', element: 'A' })).toBe('Meldung signal_dropped (A)');
  });

  test('Zeilen aller Bereiche, neueste zuerst', () => {
    const rows = reviewRows([
      { area: 'Adorf', cycles: [{ cycle: 1, now_ms: 100, commands: [{ type: 'reset_tvd', tvd: 'T1' }], events: [] }] },
      { area: 'Bdorf', cycles: [{ cycle: 3, now_ms: 300, commands: [], events: [{ type: 'accepted', request: 2 }] }] },
    ]);
    expect(rows.map((r) => [r.now_ms, r.area, r.kind])).toEqual([
      [300, 'Bdorf', 'event'],
      [100, 'Adorf', 'command'],
    ]);
  });
});
