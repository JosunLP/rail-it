// Erzeugt die Programmreferenz (docs/referenz/programme/*.md) aus den Hilfetexten der
// Programme. Die Seiten werden beim Bauen der Dokumentation erzeugt und nicht eingecheckt;
// so passen sie immer zum Stand des Quelltexts.
//
//   bun run gen                       # Programme aus target/debug (sonst cargo build)
//   RAILIT_BIN=/pfad/zu/bin bun run gen

import { existsSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const root = join(import.meta.dir, '..', '..');
const out = join(import.meta.dir, '..', 'referenz', 'programme');
const exe = process.platform === 'win32' ? '.exe' : '';
const programs = [
  { name: 'railit', title: 'railit – Kommandozeile', intro: 'Netze und Pläne prüfen, Szenarien ausführen, Journale auswerten, railitd bedienen und die Installation pflegen.' },
  { name: 'railitd', title: 'railitd – Stellwerksdienst', intro: 'Kerne, Feld und Betriebsführung im festen Zyklus; REST/JSON, gRPC, Ereignisstrom und Bedienplatz.' },
  { name: 'railit-field', title: 'railit-field – Feldrechner', intro: 'Simulierte Objektsteuerungen hinter EULYNX SCI und RaSTA über UDP, mit Umschalter zwischen Haupt- und Bereitschaftsrechner.' },
  { name: 'railit-node', title: 'railit-node – Knoten des sicheren Rechners', intro: 'Kern mit redundanten Kanälen, Voter und Watchdog im eigenen Takt, angebunden über RaSTA.' },
];

function binDir(): string {
  if (process.env.RAILIT_BIN) return process.env.RAILIT_BIN;
  const debug = join(root, 'target', 'debug');
  if (!programs.every((p) => existsSync(join(debug, p.name + exe)))) {
    const args = ['build', '-q', ...programs.flatMap((p) => ['-p', p.name])];
    const r = Bun.spawnSync(['cargo', ...args], { cwd: root, stdout: 'inherit', stderr: 'inherit' });
    if (r.exitCode !== 0) throw new Error('cargo build fehlgeschlagen');
  }
  return debug;
}

function help(bin: string, args: string[]): string {
  const r = Bun.spawnSync([bin, ...args, '--help'], { env: { ...process.env, NO_COLOR: '1' } });
  if (r.exitCode !== 0) throw new Error(`${args.join(' ')} --help: ${r.stderr.toString()}`);
  return r.stdout.toString().trimEnd();
}

/** Unterbefehle aus dem Abschnitt „Commands:“ eines Hilfetexts. */
function subcommands(text: string): string[] {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => l.trim() === 'Commands:');
  if (start < 0) return [];
  const names: string[] = [];
  for (const line of lines.slice(start + 1)) {
    if (!line.trim()) break;
    const m = /^ {2}([a-z][a-z0-9-]*)\s/.exec(line);
    if (m && m[1] !== 'help') names.push(m[1]);
  }
  return names;
}

/** Optionsblöcke des Abschnitts „Options:“ (je Option ihr vollständiger Text). */
function optionBlocks(text: string): string[] {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => l.trim() === 'Options:');
  if (start < 0) return [];
  const blocks: string[][] = [];
  for (const line of lines.slice(start + 1)) {
    if (/^\s{2,6}-/.test(line)) blocks.push([line]);
    else if (blocks.length) blocks[blocks.length - 1].push(line);
  }
  return blocks.map((b) => b.join('\n').trimEnd());
}

/** Optionen des Aufrufs von `railit`, die jeder Befehl kennt; bei Befehlen nicht wiederholt. */
let globals = new Set<string>();

/** Entfernt die globalen Optionen aus dem Abschnitt „Options:“ eines Befehls. */
function withoutGlobals(text: string): string {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => l.trim() === 'Options:');
  if (start < 0) return text;
  const kept = optionBlocks(text).filter((b) => !globals.has(b));
  const head = lines.slice(0, start).join('\n').trimEnd();
  return kept.length ? `${head}\n\nOptions:\n${kept.join('\n\n')}` : head;
}

/** Erste Zeile eines Hilfetexts (Beschreibung). */
function summary(text: string): string {
  return text.split('\n')[0] ?? '';
}

function section(bin: string, name: string, path: string[], depth: number): string {
  const full = help(bin, path);
  const text = name === 'railit' ? withoutGlobals(full) : full;
  const heading = '#'.repeat(Math.min(depth, 4));
  let md = `${heading} \`${[name, ...path].join(' ')}\`\n\n${summary(text)}\n\n\`\`\`text\n${text}\n\`\`\`\n\n`;
  for (const sub of subcommands(full)) md += section(bin, name, [...path, sub], depth + 1);
  return md;
}

const dir = binDir();
await mkdir(out, { recursive: true });
for (const p of programs) {
  const bin = join(dir, p.name + exe);
  const version = Bun.spawnSync([bin, '--version']).stdout.toString().trim();
  const text = help(bin, []);
  globals = new Set(optionBlocks(text));
  let md = `---\noutline: [2, 3]\n---\n\n# ${p.title}\n\n${p.intro}\n\n`;
  md += `::: info Erzeugt\nDiese Seite entsteht beim Bauen der Dokumentation aus \`${p.name} --help\` (${version}).\n:::\n\n`;
  md += `## Aufruf\n\n\`\`\`text\n${text}\n\`\`\`\n\n`;
  const subs = subcommands(text);
  if (subs.length) {
    md += '## Befehle\n\n';
    if (p.name === 'railit') {
      md += 'Die Optionen unter *Aufruf* (Ausgabeformat, Server, Anmeldung, TLS) gelten für jeden Befehl und sind unten nicht wiederholt.\n\n';
    }
    for (const sub of subs) md += section(bin, p.name, [sub], 3);
  }
  await writeFile(join(out, `${p.name}.md`), md);
  console.log(`referenz/programme/${p.name}.md`);
}
