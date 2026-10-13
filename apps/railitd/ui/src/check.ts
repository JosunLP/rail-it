// Prüfcode der sicheren Anzeige- und Bestätigungskette (REQ-UI-003).
//
// Der Bedienplatz berechnet den Code aus dem, was er dem Bediener anzeigt. Der Dienst gibt
// eine Bestätigung nur weiter, wenn dieser Code mit dem Code der offenen Hilfshandlung des
// Kerns übereinstimmt. Das Verfahren ist dasselbe wie `railit_runtime::aux_check`.

import type { NamedAux } from './types';

/** Kanonischer Text einer Hilfshandlung, so wie sie angezeigt wird. */
export function canonical(area: string, aux: number, op: NamedAux): string {
  return ['railit-aux-v1', area, String(aux), op.kind, op.element, op.position ?? ''].join('\n');
}

/** Prüfcode: die ersten drei Bytes des SHA-256 als sechs Großbuchstaben-Hexziffern. */
export async function auxCheck(area: string, aux: number, op: NamedAux): Promise<string> {
  const data = new TextEncoder().encode(canonical(area, aux, op));
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', data));
  return Array.from(digest.slice(0, 3), (b) => b.toString(16).padStart(2, '0'))
    .join('')
    .toUpperCase();
}
