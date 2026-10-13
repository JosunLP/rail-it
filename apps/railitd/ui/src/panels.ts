// Bedienfenster als bQuery-Web-Components (Light DOM, Gestaltung in style.css).
//
// Alle Inhalte entstehen mit `safeHtml`: eingesetzte Werte (Namen aus dem Netz, Meldungen)
// werden maskiert und können kein Markup erzeugen.

import { bool, component, safeHtml } from '@bquery/bquery/component';
import { computed, signal } from '@bquery/bquery/reactive';
import { type SanitizedHtml, trusted } from '@bquery/bquery/security';
import { auxText, clock, type DisplayHealth, nodeProblem, trainText } from './model';
import type { OpsState, PendingAux, ReviewRow, Snapshot } from './types';

/** Aktuelles Zustandsbild (undefiniert bis zum ersten Abruf). */
export const snapshot = signal<Snapshot | undefined>(undefined);
/** Zustand der Anzeige. */
export const displayHealth = signal<DisplayHealth>('stale');
/** Zeitpunkt der Rückschau in Millisekunden; `undefined`: Betrieb (keine Rückschau). */
export const reviewAt = signal<number | undefined>(undefined);
/** Der Dienst ist ein Bereitschaftsrechner: Anzeige ohne Bedienung (REQ-SVC-011). */
export const standbyRole = signal(false);

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
    const problem = snap ? nodeProblem(snap) : undefined;
    const areas = (snap?.areas ?? []).map((a) => {
      const active = a.safety.channels.filter((c) => c.active).length;
      const total = a.safety.channels.length;
      const tripped = a.safety.trip !== null && a.safety.trip !== undefined;
      const cls = tripped ? 'badge error' : active < total ? 'badge warn' : 'badge';
      const text = tripped ? 'abgeschaltet' : `${active}/${total} Kanäle`;
      const n = a.safety.node;
      const node = !n
        ? ''
        : !n.available
          ? ' · Knoten getrennt'
          : !n.synced
            ? ' · Abgleich'
            : ' · Knoten';
      const nodeCls = n && (!n.available || !n.synced) ? 'badge error' : cls;
      const title = n
        ? `Sicherer Rechner ${n.link}: ${n.cycles} Zyklen, ${n.syncs} Abgleiche, ${n.divergences} Abweichungen, ${n.link_losses} Abbrüche`
        : 'Sicherheitsschicht';
      return safeHtml`<span class="${nodeCls}" title="${title}" data-node="${n ? (n.available && n.synced ? 'ok' : 'down') : ''}">${a.name}: ${text}${node}</span>`;
    });
    return safeHtml`
      <span class="network">${snap?.network ?? '…'}</span>
      <span class="clock">${snap ? clock(snap.time_ms) : '–'}</span>
      <span class="cycle">Zyklus ${snap?.cycle ?? '–'}</span>
      <span class="badge health-${health}" data-health="${health}">${HEALTH_TEXT[health]}</span>
      ${trusted(problem ? safeHtml`<span class="node-problem">${problem}</span>` : safeHtml``)}
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
  // In der Rückschau wird nicht bedient.
  operable: displayHealth.value !== 'stale' && reviewAt.value === undefined && !standbyRole.value,
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

interface TrainsView {
  ops: OpsState | null;
  operable: boolean;
}

const trains = view<TrainsView>(() => ({
  ops: snapshot.value?.ops ?? null,
  operable: displayHealth.value !== 'stale',
}));

/** Züge mit Zugnummer, Zuglenkung und Disposition (Betriebsführung). */
component('rit-trains', {
  shadow: false,
  signals: { trains },
  render: ({ signals }) => {
    const { ops, operable } = JSON.parse(signals.trains.value) as TrainsView;
    if (!ops) return safeHtml`<h2>Züge</h2><p class="empty">Betriebsführung aus</p>`;
    const rows = ops.trains
      .filter((t) => !t.left)
      .map(
        (t) => safeHtml`
          <li class="${t.hold ? 'train held' : 'train'}" data-train="${t.number}">
            <span><b>${t.number}</b> ${trainText(t)}</span>
            <button type="button" class="hold" data-hold="${t.number}" data-on="${t.hold ? 'false' : 'true'}" ${bool('disabled', !operable)}>${t.hold ? 'Freigeben' : 'Zurückhalten'}</button>
          </li>`,
      );
    return safeHtml`
      <h2>Züge
        <button type="button" class="${ops.ars ? 'ars on' : 'ars'}" data-ars="${ops.ars ? 'false' : 'true'}" title="Zuglenkung ein- oder ausschalten" ${bool('disabled', !operable)}>${ops.ars ? 'Zuglenkung ein' : 'Zuglenkung aus'}</button>
      </h2>
      ${trusted(rows.length ? safeHtml`<ul>${join(rows)}</ul>` : safeHtml`<p class="empty">keine Züge</p>`)}
    `;
  },
});

// ── Rückschau (REQ-UI-005) ──────────────────────────────────────────────────────────────

/** Zeilen der Rückschau: Befehle und Ereignisse der letzten Minute vor `reviewAt`. */
export const reviewRowsSignal = signal<ReviewRow[]>([]);

const reviewView = view(() => ({ at: reviewAt.value, rows: reviewRowsSignal.value.slice(0, 200) }));

/** Befehle und Ereignisse vor dem Zeitpunkt der Rückschau. */
component('rit-review', {
  shadow: false,
  signals: { reviewView },
  render: ({ signals }) => {
    const { at, rows } = JSON.parse(signals.reviewView.value) as { at?: number; rows: ReviewRow[] };
    if (at === undefined) return safeHtml``;
    const items = rows.map(
      (r) => safeHtml`
        <li class="review-${r.kind}">
          <span class="time">${clock(r.now_ms)}</span>
          <span class="area">${r.area}</span>
          <span class="text">${r.kind === 'command' ? '→ ' : ''}${r.text}</span>
        </li>`,
    );
    return safeHtml`
      <h2>Rückschau bis ${clock(at)}</h2>
      ${trusted(items.length ? safeHtml`<ul>${join(items)}</ul>` : safeHtml`<p class="empty">keine Befehle oder Ereignisse in der Minute davor</p>`)}
    `;
  },
});
