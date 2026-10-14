# Releases

Releases entstehen vollständig in GitHub Actions (`.github/workflows/release.yml`). Für jede
Plattform wird der Bedienplatz in der Pipeline gebaut und in `railitd` eingebettet;
gebaute Dateien liegen nie im Repository.

## Ein Release anlegen

**Actions → Release → Run workflow**, Version ohne `v` eingeben (z. B. `0.11.0`, Vorabversion
`0.11.0-rc.1`). Der Workflow

1. hebt mit `cargo xtask release <Version>` die Version in `Cargo.toml`, `Cargo.lock`,
   `apps/railitd/ui/package.json` und `docs/package.json` an und schreibt in `CHANGELOG.md`
   einen Abschnitt mit allen Commits seit dem letzten Tag,
2. committet das als `Release v<Version>` und setzt den Tag `v<Version>`,
3. baut für alle Ziele, testet die Programme und den Installer gegen jedes Archiv,
4. veröffentlicht das GitHub-Release mit Archiven, `install.sh`, `install.ps1`, `SHA256SUMS`
   und dem Abschnitt aus dem Changelog als Text.

Alternativ: Version selbst anheben, committen und einen Tag `vX.Y.Z` pushen. Der Workflow
prüft dann, dass Tag, `Cargo.toml` und `CHANGELOG.md` zusammenpassen.

Vorab lokal ansehen:

```sh
cargo xtask release 0.11.0 --dry-run   # zeigt neuen Abschnitt des Changelogs
cargo xtask notes 0.11.0               # Text des Releases aus CHANGELOG.md
```

Die Commits werden nach ihrem Betreff eingeordnet: `fix…`, „Fehler“ und „behoben“ unter
*Behoben*, `feat…` und Phasen (`P10-3: …`, `Phase 4: …`) unter *Neu*, `docs…` unter
*Dokumentation*, alles andere unter *Weiteres*.

## Ziele

| Ziel | Runner | Archiv |
|------|--------|--------|
| `x86_64-unknown-linux-musl` | ubuntu-latest | `railit-x86_64-unknown-linux-musl.tar.gz` |
| `aarch64-unknown-linux-musl` | ubuntu-24.04-arm | `railit-aarch64-unknown-linux-musl.tar.gz` |
| `aarch64-apple-darwin` | macos-latest | `railit-aarch64-apple-darwin.tar.gz` |
| `x86_64-apple-darwin` | macos-latest (Kreuzbau) | `railit-x86_64-apple-darwin.tar.gz` |
| `x86_64-pc-windows-msvc` | windows-latest | `railit-x86_64-pc-windows-msvc.zip` |

Linux-Programme sind statisch mit musl gebunden und laufen auf jeder Distribution. Jedes
Archiv enthält `railit`, `railitd`, `railit-field`, `railit-node`, die Beispielnetze,
`LICENSE`, `README.md`, `CHANGELOG.md`, `VERSION` und den Installer. Die Archivnamen tragen
keine Version, damit `releases/latest/download/…` immer auf das neueste Release zeigt.

## Installer

`scripts/install.sh` (Linux, macOS) und `scripts/install.ps1` (Windows) installieren,
aktualisieren und entfernen; die Dokumentation veröffentlicht beide unter
`https://josunlp.github.io/rail-it/`. Jede Datei wird gegen `SHA256SUMS` geprüft; ein Manifest
unter `<Prefix>/share/rail-it/manifest` hält fest, was installiert ist. `railit self update`
und `railit self uninstall` rufen den mitinstallierten Installer auf.

Getestet werden die Installer mit `scripts/test-installer.sh` und `scripts/test-installer.ps1`
– in der CI gegen ein nachgebautes Release auf einem lokalen HTTP-Server, im Release-Workflow gegen jedes
gebaute Archiv. Benutzung: [Installation](../leitfaden/installation).
