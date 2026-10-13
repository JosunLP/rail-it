// Baut den Bedienplatz nach `dist/`: ein Modul (`app.js`), `index.html`, `style.css`.
// `dist/` ist eingecheckt, damit railitd ohne Bun gebaut werden kann; CI prüft, dass es
// zum Quelltext passt.

import { copyFile, mkdir, rm } from 'node:fs/promises';
import { join } from 'node:path';

const root = join(import.meta.dir, '..');
const dist = join(root, 'dist');

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

const result = await Bun.build({
  entrypoints: [join(root, 'src/main.ts')],
  outdir: dist,
  naming: 'app.js',
  target: 'browser',
  format: 'esm',
  minify: true,
  sourcemap: 'none',
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

for (const file of ['index.html', 'style.css']) {
  await copyFile(join(root, 'src', file), join(dist, file));
}

for (const out of result.outputs) {
  console.log(`${out.path.slice(root.length + 1)}  ${(out.size / 1024).toFixed(1)} KiB`);
}
