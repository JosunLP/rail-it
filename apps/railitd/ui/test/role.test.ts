// Rolle des Dienstes und Übernahme (REQ-SVC-011, REQ-SVC-014): reine Funktionen.

import { describe, expect, test } from 'bun:test';
import { roleOf, takeoverText } from '../src/model';

describe('roleOf', () => {
  test('Hauptrechner ohne Bereitschaft', () => {
    expect(roleOf({ role: 'primary', standby: null })).toEqual({ role: 'primary' });
    expect(roleOf(undefined)).toEqual({ role: 'primary' });
  });

  test('Bereitschaftsrechner liest mit', () => {
    const h = { role: 'standby', standby: { primary: 'http://a:7878', phase: { phase: 'following' } } };
    expect(roleOf(h)).toEqual({ role: 'standby', primary: 'http://a:7878' });
  });

  test('während der Übernahme bleibt die Bedienung gesperrt', () => {
    const h = { role: 'standby', standby: { primary: 'x', phase: { phase: 'acquiring', since_ms: 5 } } };
    expect(roleOf(h).role).toBe('standby');
  });

  test('nach der Übernahme führt der Rechner', () => {
    const h = {
      role: 'primary',
      standby: { primary: 'x', phase: { phase: 'leading', warm: true, at_ms: 9, detail: 'gedeckt' } },
    };
    const r = roleOf(h);
    expect(r).toEqual({ role: 'primary', takeover: { warm: true, detail: 'gedeckt' } });
    expect(takeoverText(r.takeover!)).toContain('mitgelesenen Zustand');
    expect(takeoverText({ warm: false, detail: 'keine Zusage' })).toContain('Neustart');
  });
});
