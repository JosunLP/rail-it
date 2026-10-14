# Stellwerksdienst

`railitd` lädt Netz und Plan, prüft Signatur und Zusammengehörigkeit, betreibt Kerne, Feld und
Betriebsführung im festen Zyklus und stellt die API und den Bedienplatz bereit.

```sh
railitd --network netz.toml --plan plan.rplan --trusted-key railit.pub \
        --listen 127.0.0.1:7878 --cycle-ms 100
```

Die erste Zeile auf stdout nennt die Adresse (`railitd: http://127.0.0.1:7878`) – praktisch für
Skripte. Strg+C beendet den Dienst; Journale werden dabei gesiegelt.

## Wichtige Optionen

| Option | Bedeutung |
|--------|-----------|
| `--network`, `--plan` | Netz und Plan (bei Stellbereichen ein Verzeichnis mit `<Bereich>.rplan`) |
| `--trusted-key` / `--allow-unsigned` | Schlüssel der Planprüfung bzw. unsignierte Pläne (nur Test) |
| `--listen` | Adresse der API (Vorgabe `127.0.0.1:7878`) |
| `--cycle-ms`, `--speed` | Zykluszeit; Zeitraffer der Simulation |
| `--field direct\|eulynx\|remote` | Feldanbindung: Simulator direkt, über EULYNX/RaSTA, Feldrechner |
| `--channels 1\|2\|3`, `--watchdog-ms` | Redundanz je Stellbereich, Watchdog |
| `--node` | Kern auf einem Knoten des sicheren Rechners |
| `--journal`, `--checkpoint-every`, `--seal-key` | Betriebsjournal, Prüfpunkte, Siegel |
| `--timetable`, `--ars` | Fahrplan, Zuglenkung |
| `--token` / `--users`, `--audit` | Anmeldung, Benutzer mit Rollen, Auditprotokoll |
| `--tls-cert`, `--tls-key`, `--tls-client-ca` | TLS und gegenseitige Anmeldung |
| `--standby`, `--takeover-after-ms`, `--standby-gate-ms` | Bereitschaftsrechner und Übernahme |

Alle Optionen mit Beschreibung: [railitd](/referenz/programme/railitd).

## Schnittstellen

| Pfad | Inhalt |
|------|--------|
| `/ui/` | Bedienplatz ([Kapitel](./bedienplatz)) |
| `/v1/state`, `/v1/layout`, `/v1/plan` | Zustandsbild, Spurplan, Plan |
| `/v1/routes/…`, `/v1/points/…`, `/v1/aux` | Bedienung |
| `/v1/events` | Ereignisstrom (Server-Sent Events) |
| `/v1/ops/…`, `/v1/sim/…`, `/v1/safety/…` | Betriebsführung, Simulation, Sicherheitsschicht |
| `/v1/journal/…` | Rückschau und Journalstrom |
| `/v1/metrics` | Metriken (Prometheus) |
| gRPC `railit.v1` | auf demselben Port |

Die vollständige Beschreibung steht in [REST-API und gRPC](/api).

## Ohne Anmeldung nur lokal

Ohne `--token` oder `--users` lauscht der Dienst nur auf der Loopback-Adresse; jeder lokale
Zugriff hat dann alle Rechte. Für den Mehrbenutzerbetrieb siehe
[Benutzer, Rollen, TLS](./zugang).
