# Umgebungsvariablen

## Programme

| Variable | Programm | Entspricht | Bedeutung |
|----------|----------|------------|-----------|
| `RAILIT_SERVER` | `railit` | `--server` | Adresse von `railitd` (Vorgabe `http://127.0.0.1:7878`) |
| `RAILIT_TOKEN` | `railit` | `--token` | Zugangstoken für `railitd` |
| `RAILIT_CA` | `railit` | `--ca` | CA des Serverzertifikats (PEM) |
| `RAILIT_CERT`, `RAILIT_KEY` | `railit` | `--cert`, `--key` | Client-Zertifikat und Schlüssel (mTLS) |
| `RAILIT_TOKEN` | `railitd` | `--token` | Gemeinsames Token mit allen Rechten (schließt `--users` aus) |
| `RAILIT_FIELD_TOKEN` | `railitd` | `--field-token` | Token der Steuerung des Feldrechners |
| `RAILIT_FIELD_TOKEN` | `railit-field` | `--token` | Token der Simulationssteuerung |

Token-Werte erscheinen nicht in der Hilfe.

## Installation und `railit self`

| Variable | Bedeutung |
|----------|-----------|
| `RAILIT_PREFIX` | Installationsverzeichnis (Programme in `bin`, Daten in `share/rail-it`) |
| `RAILIT_VERSION` | Bestimmte Version statt der neuesten (`install.sh`, `install.ps1`) |
| `RAILIT_REPO` | Repository der Releases (Vorgabe `JosunLP/rail-it`) |
| `RAILIT_DOWNLOAD_URL` | Basis der Downloads statt `https://github.com/<Repo>/releases`, z. B. ein Spiegel |
| `RAILIT_RELEASES_API` | Adresse für die neueste Version (`railit self check`), Vorgabe die GitHub-API |

## Bauen und Prüfen

| Variable | Bedeutung |
|----------|-----------|
| `RAILIT_UI_DIST` | Verzeichnis mit dem gebauten Bedienplatz (Vorgabe `apps/railitd/ui/dist`) |
| `RAILIT_REQUIRE_UI` | `1`: Bauen von `railitd` scheitert ohne gebauten Bedienplatz statt einen Platzhalter einzubetten (CI, Releases) |
| `RAILIT_BIN` | Verzeichnis der Programme für `bun run gen` in `docs/` |
| `PROPTEST_CASES` | Fälle je Property-Test |
| `TAKEOVER_CASES` | Fälle der Übernahme-Tests in `railit-runtime` |
