import { describe, expect, test } from 'bun:test';
import { auxCheck, canonical } from '../src/check';

// Dieselben Prüfvektoren prüft `railit-runtime` (names.rs, `check_code_vectors`).
describe('Prüfcode (REQ-UI-003)', () => {
  test('Vektoren wie im Dienst', async () => {
    expect(await auxCheck('Beispielstadt', 0, { kind: 'release_route', element: 'A-N1' })).toBe('63CC78');
    expect(await auxCheck('Adorf', 7, { kind: 'force_point', element: 'W1', position: 'left' })).toBe('98171F');
    expect(await auxCheck('Bstadt', 12, { kind: 'reset_tvd', element: 'Ü1' })).toBe('DF8640');
  });

  test('jede Abweichung ändert den Code', async () => {
    const base = await auxCheck('A', 1, { kind: 'release_route', element: 'X' });
    expect(await auxCheck('A', 2, { kind: 'release_route', element: 'X' })).not.toBe(base);
    expect(await auxCheck('A', 1, { kind: 'substitute', element: 'X' })).not.toBe(base);
    expect(await auxCheck('A', 1, { kind: 'release_route', element: 'Y' })).not.toBe(base);
    expect(await auxCheck('B', 1, { kind: 'release_route', element: 'X' })).not.toBe(base);
  });

  test('kanonischer Text', () => {
    expect(canonical('A', 3, { kind: 'force_point', element: 'W2', position: 'right' })).toBe(
      'railit-aux-v1\nA\n3\nforce_point\nW2\nright',
    );
  });
});
