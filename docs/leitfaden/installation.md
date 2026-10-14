# Installation

## Mit dem Installer

Der Installer lädt die Programme des neuesten Releases, prüft sie gegen die Prüfsummen des
Releases und legt sie in Ihr Benutzerverzeichnis.

::: code-group

```sh [Linux / macOS]
curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh
```

```powershell [Windows]
irm https://josunlp.github.io/rail-it/install.ps1 | iex
```

:::

Installiert werden `railit` (Kommandozeile), `railitd` (Stellwerksdienst mit Bedienplatz),
`railit-field` (Feldrechner) und `railit-node` (Knoten des sicheren Rechners), dazu die
Beispielnetze.

| | Linux / macOS | Windows |
|---|---|---|
| Programme | `~/.local/bin` (als root `/usr/local/bin`) | `%LOCALAPPDATA%\Programs\rail-it\bin` |
| Beispielnetze, Lizenz | `~/.local/share/rail-it` | `%LOCALAPPDATA%\Programs\rail-it\share\rail-it` |
| PATH | Hinweis, falls das Verzeichnis fehlt | wird für den Benutzer eingetragen |

Unterstützt werden Linux auf x86_64 und aarch64 (statisch gebaut, ohne Abhängigkeiten), macOS
auf Intel und Apple Silicon sowie Windows auf x86_64.

### Optionen

```sh
# bestimmte Version
curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- --version 0.11.0
# anderes Präfix (Programme nach /opt/rail-it/bin)
curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- --prefix /opt/rail-it
```

| Option | Umgebungsvariable | Bedeutung |
|--------|-------------------|-----------|
| `--version X.Y.Z` | `RAILIT_VERSION` | bestimmte Version statt der neuesten |
| `--prefix DIR` | `RAILIT_PREFIX` | Programme nach `DIR/bin`, Daten nach `DIR/share/rail-it` |
| `--force` | | auch installieren, wenn die Version schon installiert ist |
| `--no-modify-path` | | keinen Hinweis zum PATH ausgeben |
| | `RAILIT_REPO` | anderes GitHub-Repository (Vorgabe `JosunLP/rail-it`) |
| | `RAILIT_DOWNLOAD_URL` | andere Quelle der Releases (Spiegel, Prüfstand ohne Internet) |

Unter Windows heißen die Parameter `-Version`, `-Prefix` und `-Force`.

## Aktualisieren

```sh
railit self check     # gibt es ein neueres Release? (Exit-Code 1, wenn ja)
railit self update    # auf das neueste Release aktualisieren
railit self update --version 0.11.0
```

`railit self update` ruft den mitinstallierten Installer auf; laufende Programme werden dabei
nicht abgebrochen, sondern ersetzt. Ohne vorhandenes `railit` geht es auch so:

::: code-group

```sh [Linux / macOS]
curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- update
```

```powershell [Windows]
& ([scriptblock]::Create((irm https://josunlp.github.io/rail-it/install.ps1))) update
```

:::

## Entfernen

```sh
railit self uninstall
```

Der Installer führt ein Manifest aller installierten Dateien
(`…/share/rail-it/manifest`) und entfernt genau diese – Ihre Netze, Pläne und Journale bleiben
unberührt. Unter Windows wird auch der Eintrag im PATH entfernt. Alternativ:

::: code-group

```sh [Linux / macOS]
curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- uninstall
```

```powershell [Windows]
& ([scriptblock]::Create((irm https://josunlp.github.io/rail-it/install.ps1))) uninstall
```

:::

## Von Hand

Jedes [Release](https://github.com/JosunLP/rail-it/releases) enthält je Plattform ein Archiv
(`railit-<Ziel>.tar.gz` bzw. `.zip`) und `SHA256SUMS`:

```sh
curl -fsSLO https://github.com/JosunLP/rail-it/releases/latest/download/railit-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO https://github.com/JosunLP/rail-it/releases/latest/download/SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf railit-x86_64-unknown-linux-musl.tar.gz
```

## Aus dem Quelltext

Voraussetzungen: Rust 1.97 (wird über `rust-toolchain.toml` gewählt) und für den Bedienplatz
[Bun](https://bun.sh) ab 1.4.

```sh
git clone https://github.com/JosunLP/rail-it.git && cd rail-it
(cd apps/railitd/ui && bun install && bun run build)   # Bedienplatz
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

Ohne gebauten Bedienplatz entsteht `railitd` trotzdem; unter `/ui/` erscheint dann eine
Hinweisseite. Mit `RAILIT_REQUIRE_UI=1` bricht der Build stattdessen ab – so bauen CI und
Release.
