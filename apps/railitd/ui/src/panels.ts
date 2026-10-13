// Bedienfenster als bQuery-Web-Components (Light DOM, Gestaltung in style.css).
//
// Alle Inhalte entstehen mit `safeHtml`: eingesetzte Werte (Namen aus dem Netz, Meldungen)
// werden maskiert und können kein Markup erzeugen.

import { bool, component, safeHtml } from '@bquery/bquery/component';
import { computed, signal } from '@bquery/bquery/reactive';
import { type SanitizedHtml, trusted } from '@bquery/bquery/security';
import { auxText, clock, type DisplayHealth } from './model';
import type { PendingAux, Snapshot } from './types';

/** Aktuelles Zustandsbild (undefiniert bis zum ersten Abruf). */
export const snapshot = signal<Snapshot | undefined>(undefined);
/** Zustand der Anzeige. */
export const displayHealth = signal<DisplayHealth>('stale');

export interface Message {
  time: string;
  level: 'info' | 'warn' | 'error';
  text: string;
}

/** Meldungen, neueste zuerst. */
export const messages = signal<Message[]>([]);

export function report(level: Message['level'], text: string): void {
  const s = snapshot.peek();
  const time = s ? clock(s.time_ms) : '–';
  messages.value = [{ time, level, text }, ...messages.peek()].slice(0, 80);
}

/**
 * Setzt Stücke zusammen, die alle aus `safeHtml` stammen und damit bereits maskiert sind.
 * Nur deshalb darf das Ergebnis als vertrauenswürdig gelten.
 */
const join = (parts: SanitizedHtml[]) => trusted(parts.join('') as SanitizedHtml);

const HEALTH_TEXT: Record<DisplayHealth, string> = {
  ok: 'Anzeige aktuell',
  stale: 'ANZEIGE GESTÖRT – keine Bedienung',
  safe_state: 'SICHERER ZUSTAND',
};

const status = computed(() => ({ snap: snapshot.value, health: displayHealth.value }));

/** Kopfzeile: Netz, Zeit, Zyklus, Anzeige, Sicherheitsschicht. */
component('rit-status', {
  shadow: false,
  signals: { status },
  render: ({ signals }) => {
    const { snap, health } = signals.status.value;
    const areas = (snap?.areas ?? []).map((a) => {
      const active = a.safety.channels.filter((c) => c.active).length;
      const total = a.safety.channels.length;
      const tripped = a.safety.trip !== null && a.safety.trip !== undefined;
      const cls = tripped ? 'badge error' : active < total ? 'badge warn' : 'badge';
      const text = tripped ? 'abgeschaltet' : `${active}/${total} Kanäle`;
      return safeHtml`<span class="${cls}" title="Sicherheitsschicht">${a.name}: ${text}</span>`;
    });
    return safeHtml`
      <span class="network">${snap?.network ?? '…'}</span>
      <span class="clock">${snap ? clock(snap.time_ms) : '–'}</span>
      <span class="cycle">Zyklus ${snap?.cycle ?? '–'}</span>
      <span class="badge health-${health}" data-health="${health}">${HEALTH_TEXT[health]}</span>
      ${join(areas)}
    `;
  },
});

/**
 * Sicht als Zeichenkette: `computed` meldet nur bei geänderter Sicht, damit Schaltflächen nicht
 * bei jedem Zustandsbild (alle 300 ms) neu entstehen (Fokus, Hover, Klick bleiben erhalten).
 */
function view<T>(read: () => T) {
  return computed(() => JSON.stringify(read()));
}

interface AuxView {
  list: PendingAux[];
  counter: number;
  operable: boolean;
}

const pending = view<AuxView>(() => ({
  list: snapshot.value?.pending_aux ?? [],
  counter: snapshot.value?.aux_counter ?? 0,
  operable: displayHealth.value !== 'stale',
}));

/** Offene Hilfshandlungen mit Bestätigung und Zählwerk. */
component('rit-aux', {
  shadow: false,
  signals: { pending },
  render: ({ signals }) => {
    const { list, counter, operable } = JSON.parse(signals.pending.value) as AuxView;
    const rows = list.map(
      (p) => safeHtml`
        <li>
          <span class="aux-text">#${p.aux} ${auxText(p.op)}</span>
          <span class="aux-meta">${p.area} · Bediener ${p.operator}</span>
          <button type="button" class="confirm" data-aux="${p.aux}" data-area="${p.area}" ${bool('disabled', !operable)}>Bestätigen …</button>
        </li>`,
    );
    return safeHtml`
      <h2>Hilfshandlungen <span class="counter" title="Zählwerk">Zählwerk ${counter}</span></h2>
      ${trusted(rows.length ? safeHtml`<ul>${join(rows)}</ul>` : safeHtml`<p class="empty">keine offen</p>`)}
    `;
  },
});

/** Meldungen. */
component('rit-log', {
  shadow: false,
  signals: { messages },
  render: ({ signals }) => {
    const rows = signals.messages.value.map(
      (m) => safeHtml`<li class="msg-${m.level}"><time>${m.time}</time> ${m.text}</li>`,
    );
    return safeHtml`<h2>Meldungen</h2><ul>${join(rows)}</ul>`;
  },
});

const trains = view(() => snapshot.value?.ops ?? null);

/** Züge mit Zugnummer (Betriebsführung). */
component('rit-trains', {
  shadow: false,
  signals: { trains },
  render: ({ signals }) => {
    const ops = JSON.parse(signals.trains.value) as Snapshot['ops'] | null;
    if (!ops) return safeHtml`<h2>Züge</h2><p class="empty">Betriebsführung aus</p>`;
    const rows = ops.trains
      .filter((t) => !t.left)
      .map((t) => {
        const delay = t.delay_s > 0 ? `+${Math.round(t.delay_s / 60)} min` : 'pünktlich';
        return safeHtml`<li><b>${t.number}</b> an ${t.berth ?? '–'} · ${delay}</li>`;
      });
    return safeHtml`<h2>Züge <span class="badge">${ops.ars ? 'Zuglenkung ein' : 'Zuglenkung aus'}</span></h2><ul>${join(rows)}</ul>`;
  },
});
