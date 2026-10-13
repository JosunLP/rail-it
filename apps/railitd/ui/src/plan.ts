// Spurplan als SVG (REQ-UI-001): einmal aus der Geometrie aufgebaut, danach nur über Klassen aktualisiert.
//
// Die Elemente entstehen mit `createElementNS`, nie aus HTML-Zeichenketten: Namen aus dem
// Netz werden ausschließlich als Text gesetzt.

import { $ } from '@bquery/bquery/core';
import { berths, pointPosition, signalClass, tvdClass } from './model';
import type { Layout, Pos, Snapshot } from './types';

const NS = 'http://www.w3.org/2000/svg';

type Attrs = Record<string, string | number>;

function el<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Attrs = {}, parent?: Element): SVGElementTagNameMap[K] {
  const node = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  parent?.appendChild(node);
  return node;
}

function text(parent: Element, at: Pos, content: string, cls: string): SVGTextElement {
  const t = el('text', { x: at[0], y: at[1], class: cls }, parent);
  t.textContent = content;
  return t;
}

function segment(parent: Element, a: Pos, b: Pos, cls: string): SVGLineElement {
  return el('line', { x1: a[0], y1: a[1], x2: b[0], y2: b[1], class: cls }, parent);
}

/** Wohin der Bediener im Spurplan geklickt hat. */
export type PlanTarget =
  | { kind: 'signal'; name: string }
  | { kind: 'point'; name: string }
  | { kind: 'track'; tvd: string }
  | { kind: 'border'; left: string; right: string }
  | { kind: 'empty' };

interface Handlers {
  /** Linksklick. */
  select(target: PlanTarget): void;
  /** Rechtsklick: Kontextmenü an der Mausposition. */
  menu(target: PlanTarget, x: number, y: number): void;
}

export class PlanView {
  readonly svg: SVGSVGElement;
  private readonly tracks = new Map<string, SVGElement[]>();
  private readonly points = new Map<string, { group: SVGGElement; left: SVGLineElement; right: SVGLineElement; tvd: string | null }>();
  private readonly signals = new Map<string, { group: SVGGElement; berth: SVGTextElement }>();
  private readonly crossings = new Map<string, SVGGElement>();
  private readonly borders: { group: SVGGElement; left: string; right: string; arrow: SVGTextElement }[] = [];
  private selected: string | undefined;

  constructor(container: Element, layout: Layout, handlers: Handlers) {
    this.svg = el('svg', {
      viewBox: `0 0 ${layout.width} ${layout.height + 20}`,
      class: 'plan',
      role: 'img',
      'aria-label': `Spurplan ${layout.network}`,
    });
    const back = el('g', { class: 'layer-back' }, this.svg);
    const tracks = el('g', { class: 'layer-tracks' }, this.svg);
    const objects = el('g', { class: 'layer-objects' }, this.svg);

    for (const a of layout.areas) {
      text(back, [a.x0, 18], a.name, 'area-name');
    }
    for (const s of layout.stations) {
      text(back, s.at, s.name, 'station-name');
    }
    for (const b of layout.borders) {
      const group = el('g', { class: 'border', 'data-border-left': b.left, 'data-border-right': b.right }, back);
      segment(group, [b.at[0], 24], [b.at[0], layout.height + 10], 'border-line');
      el('rect', { x: b.at[0] - 12, y: b.at[1] - 44, width: 24, height: 20, class: 'border-hit' }, group);
      const arrow = text(group, [b.at[0], b.at[1] - 30], '', 'border-arrow');
      this.borders.push({ group, left: b.left, right: b.right, arrow });
    }

    for (const t of layout.tracks) {
      const [a, b] = [t.line[0], t.line[t.line.length - 1]];
      if (!a || !b) continue;
      const line = el('polyline', {
        points: t.line.map((p) => p.join(',')).join(' '),
        class: 'track',
        'data-tvd': t.tvd ?? '',
      }, tracks);
      // Unsichtbare, breite Trefferfläche für Klicks.
      el('polyline', {
        points: t.line.map((p) => p.join(',')).join(' '),
        class: 'track-hit',
        'data-tvd': t.tvd ?? '',
      }, tracks);
      text(tracks, [(a[0] + b[0]) / 2, b[1] + 22], t.name, 'track-name');
      if (t.tvd) {
        const list = this.tracks.get(t.tvd) ?? [];
        list.push(line);
        this.tracks.set(t.tvd, list);
      }
    }

    for (const p of layout.points) {
      const group = el('g', { class: 'point', 'data-point': p.name }, tracks);
      const tip = segment(group, p.tip, p.fork, 'track');
      const left = segment(group, p.fork, p.left, 'track leg');
      const right = segment(group, p.fork, p.right, 'track leg');
      el('circle', { cx: p.fork[0], cy: p.fork[1], r: 14, class: 'point-hit' }, group);
      text(group, [p.fork[0] - 10, p.fork[1] - 12], p.name, 'point-name');
      if (p.tvd) {
        const list = this.tracks.get(p.tvd) ?? [];
        list.push(tip);
        this.tracks.set(p.tvd, list);
      }
      this.points.set(p.name, { group, left, right, tvd: p.tvd });
    }

    for (const t of layout.terminals) {
      const [x, y] = t.at;
      if (t.kind === 'buffer_stop') {
        segment(tracks, [x, y - 10], [x, y + 10], 'buffer-stop');
      } else if (t.kind === 'boundary') {
        text(tracks, [t.opens === 'right' ? x - 30 : x + 8, y + 5], t.name, 'terminal-name');
      }
    }

    for (const l of layout.level_crossings) {
      const [x, y] = l.at;
      const group = el('g', { class: 'lx' }, objects);
      segment(group, [x - 6, y - 16], [x - 6, y + 16], 'lx-bar');
      segment(group, [x + 6, y - 16], [x + 6, y + 16], 'lx-bar');
      text(group, [x + 10, y - 20], l.name, 'lx-name');
      this.crossings.set(l.name, group);
    }

    for (const s of layout.signals) {
      const [x, y] = s.at;
      const right = s.facing === 'right';
      const dy = right ? 16 : -16;
      const dir = right ? 1 : -1;
      const group = el('g', { class: 'signal', 'data-signal': s.name }, objects);
      segment(group, [x, y], [x, y + dy], 'mast');
      el('polygon', {
        points: `${x},${y + dy - 7} ${x},${y + dy + 7} ${x + 13 * dir},${y + dy}`,
        class: 'lamp',
      }, group);
      el('rect', { x: x - 10, y: y + dy - 12, width: 26, height: 24, class: 'signal-hit' }, group);
      text(group, [x - 4, y + dy + (right ? 22 : -12)], s.name, 'signal-name');
      // Zugnummer hinter dem Signal, wo der Zug steht.
      const berth = text(group, [x - 12 * dir, y + dy + 4], '', right ? 'berth berth-left' : 'berth');
      this.signals.set(s.name, { group, berth });
    }

    container.replaceChildren(this.svg);
    this.bind(handlers);
  }

  private target(node: Element | null): PlanTarget {
    const signal = node?.closest('[data-signal]');
    if (signal) return { kind: 'signal', name: signal.getAttribute('data-signal') ?? '' };
    const point = node?.closest('[data-point]');
    if (point) return { kind: 'point', name: point.getAttribute('data-point') ?? '' };
    const border = node?.closest('[data-border-left]');
    if (border) {
      return {
        kind: 'border',
        left: border.getAttribute('data-border-left') ?? '',
        right: border.getAttribute('data-border-right') ?? '',
      };
    }
    const tvd = node?.getAttribute('data-tvd');
    if (tvd) return { kind: 'track', tvd };
    return { kind: 'empty' };
  }

  private bind(handlers: Handlers): void {
    const svg = $(this.svg);
    svg.on('click', (event) => handlers.select(this.target(event.target as Element)));
    svg.on('contextmenu', (event) => {
      const e = event as MouseEvent;
      e.preventDefault();
      handlers.menu(this.target(e.target as Element), e.clientX, e.clientY);
    });
  }

  /** Startsignal einer Fahrstraße hervorheben (oder Auswahl aufheben). */
  select(signal: string | undefined): void {
    if (this.selected) $(this.signals.get(this.selected)?.group ?? this.svg).removeClass('selected');
    this.selected = signal;
    const group = signal ? this.signals.get(signal)?.group : undefined;
    if (group) $(group).addClass('selected');
  }

  /** Zustandsbild übernehmen. */
  update(snapshot: Snapshot): void {
    for (const [tvd, nodes] of this.tracks) {
      const cls = tvdClass(snapshot, tvd);
      for (const n of nodes) n.setAttribute('class', `track ${cls}`);
    }
    for (const [name, p] of this.points) {
      const cls = tvdClass(snapshot, p.tvd);
      const pos = pointPosition(snapshot, name);
      const locked = snapshot.points.find((x) => x.name === name)?.locked_by ? ' locked' : '';
      p.left.setAttribute('class', `track leg ${cls} ${pos === 'left' ? 'leg-on' : 'leg-off'}`);
      p.right.setAttribute('class', `track leg ${cls} ${pos === 'right' ? 'leg-on' : 'leg-off'}`);
      p.group.setAttribute('class', `point${pos ? '' : ' point-unknown'}${locked}`);
    }
    const trains = berths(snapshot);
    for (const [name, s] of this.signals) {
      const selected = this.selected === name ? ' selected' : '';
      s.group.setAttribute('class', `signal ${signalClass(snapshot, name)}${selected}`);
      s.berth.textContent = trains.get(name) ?? '';
    }
    for (const [name, g] of this.crossings) {
      const lx = snapshot.level_crossings.find((l) => l.name === name);
      g.setAttribute('class', `lx lx-${lx?.report ?? 'unknown'}`);
    }
    for (const b of this.borders) {
      const area = snapshot.areas.find((a) => a.name === b.left);
      const border = area?.borders.find((x) => x.name === `${b.left}>${b.right}`);
      // Pfeil in Richtung der erlaubten Fahrt: Wer die Erlaubnis hält, darf hineinfahren.
      b.arrow.textContent = border?.holding ? '▶' : border?.peer_holding ? '◀' : '·';
      b.group.setAttribute('class', `border${border?.requested ? ' requested' : ''}`);
    }
  }
}
