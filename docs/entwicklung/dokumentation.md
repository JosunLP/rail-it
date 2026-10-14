# Dokumentation

Diese Seiten entstehen mit [VitePress](https://vitepress.dev/) aus `docs/` und werden bei jedem
Push auf `main` nach GitHub Pages veröffentlicht (`.github/workflows/docs.yml`).

## Lokal ansehen

```sh
cd docs
bun install
bun run gen      # Programmreferenz aus den Hilfetexten erzeugen (baut die Programme bei Bedarf)
bun run dev      # http://localhost:5173/rail-it/
bun run build    # statische Seiten nach docs/.vitepress/dist
```

## Aufbau

| Pfad | Inhalt |
|------|--------|
| `index.md` | Startseite |
| `leitfaden/` | Leitfaden: Einführung bis Übernahme |
| `referenz/` | Referenz der Formate; `referenz/programme/` wird erzeugt |
| `api.md` | REST-API und gRPC |
| `safety/`, `requirements/` | Sicherheitsnachweis und Anforderungen |
| `KONZEPT.md` | Konzept |
| `entwicklung/` | Diese Seiten |
| `aenderungen.md` | Bindet `CHANGELOG.md` ein |
| `.vitepress/config.mts` | Navigation, Seitenleisten, Suche |
| `scripts/reference.ts` | Erzeugt die Programmreferenz |

## Erzeugte Inhalte

- **Programmreferenz:** `bun run gen` ruft `--help` für jedes Programm und jeden
  Unterbefehl auf und schreibt `referenz/programme/*.md`. Die Seiten werden nicht eingecheckt
  und passen so immer zum Quelltext. `RAILIT_BIN` wählt fertige Programme.
- **Rückverfolgbarkeit:** `cargo xtask trace` schreibt `safety/traceability.md`.
- **Installer:** Der Pages-Workflow legt `scripts/install.sh` und `scripts/install.ps1` nach
  `public/`; so gilt `curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh`.

## Hinweise zum Schreiben

- Verweise innerhalb der Doku relativ und ohne `.md` (`../leitfaden/netze`).
- Verweise in den Quelltext (`../crates/…`, `../proto/…`) sind erlaubt und zeigen nur auf
  GitHub sinnvoll; der Build prüft alle anderen Verweise.
- Hinweise mit `::: tip`, `::: warning`, `::: danger`.
- Geschweifte Klammern in Fließtext außerhalb von Code als `&#123;` schreiben oder in
  `` ` `` setzen, sonst liest Vue sie als Ausdruck.
