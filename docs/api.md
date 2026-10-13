# API von `railitd` (v1)

`railitd` stellt eine REST/JSON-API, einen Ereignisstrom und – auf demselben Port – die
gRPC-Schnittstelle `railit.v1` bereit (siehe [gRPC](#grpc)). Standardadresse:
`http://127.0.0.1:7878`, mit TLS `https://…`.

## Anmeldung und Rechte

| Betriebsart | Start | Anmeldung |
|-------------|-------|-----------|
| Benutzerverwaltung | `--users users.toml --audit audit.log` | `Authorization: Bearer <token>` oder Client-Zertifikat (mTLS, `--tls-client-ca`); Rollen und Stellbereiche aus der Datei |
| Gemeinsames Token | `--token …` bzw. `RAILIT_TOKEN` | `Authorization: Bearer <token>`, alle Rechte |
| Offen | – | keine; nur auf der Loopback-Adresse zulässig |

Alle Pfade außer `/v1/health` verlangen die Anmeldung. Die Benutzerdatei:

```toml
[[user]]
name = "fdl-adorf"
roles = ["fahrdienstleiter"]
operator = 101             # Bedienernummer für Hilfshandlungen (eindeutig)
areas = ["Adorf"]          # optional: zuständige Stellbereiche
token_sha256 = "…"         # railit auth token fdl-adorf
cert_cn = "fdl-adorf"      # oder: Common Name des Client-Zertifikats
```

| Rolle | Rechte |
|-------|--------|
| `beobachter` | lesen (Zustand, Plan, Ereignisse, Betriebsführung) |
| `fahrdienstleiter` | lesen, Fahrstraßen, Weichen, Erlaubnisse, Hilfshandlungen |
| `disponent` | lesen, Betriebsführung (`/v1/ops/…`) |
| `instandhaltung` | lesen, Simulation (`/v1/sim/…`, Störungen der Feldanbindung), Auditprotokoll, Sicherheitsschicht (`/v1/safety/…`: Prüffehler, Wiederaufnahme, Rücksetzen) |
| `projektierer` | lesen, Auditprotokoll |
| `administration` | alles |

Fehlendes Recht ergibt `403`. Jede Bedienung – auch eine abgewiesene – steht im
Auditprotokoll (JSON Lines mit Hashkette, `railit audit verify audit.log`; mit
`railitd --seal-key` folgt jedem Eintrag ein Siegel, geprüft mit `--key`). TLS:
`--tls-cert server.pem --tls-key server.key`, gegenseitige Anmeldung mit
`--tls-client-ca ca.pem`.

| Methode | Pfad | Zweck | Antwort |
|---------|------|-------|---------|
| GET | `/v1/health` | Lebenszeichen | `{"status":"ok"\|"safe_state", "network", "cycle", "time_ms", "safe_state"}` |
| GET | `/`, `/ui/` | Bedienplatz (Spurplan-UI, siehe unten); ohne Anmeldung, enthält keine Daten | HTML / JS / CSS |
| GET | `/v1/layout` | Spurplan-Geometrie, aus der Topologie berechnet: Gleise, Weichen, Signale, Bahnübergänge, Grenzen, Stellbereiche, Betriebsstellen | JSON |
| GET | `/v1/plan` | Plan: Prüfsumme, Signatur, Elemente, Fahrstraßen mit Weichen, Abschnitten, Geschwindigkeit, Durchrutschweg, Flankenschutz, Bahnübergängen, Feindlichkeiten | JSON |
| GET | `/v1/state` | Zustandsbild (siehe unten) | JSON |
| GET | `/v1/metrics` | Metriken für den Betrieb (siehe unten) | Prometheus-Text |
| GET | `/v1/journal` | Journaldateien je Stellbereich (Rückschau) | JSON |
| GET | `/v1/journal/state?at_ms=…[&area=…]` | Zustandsbild zu einem Zeitpunkt, aus dem Journal | JSON |
| GET | `/v1/journal/events?from_ms=…&to_ms=…[&area=…]` | Stellbefehle und Ereignisse eines Zeitfensters (höchstens 5000 Zyklen, `truncated`) | JSON |
| POST | `/v1/routes/{name}/set` | Fahrstraße einstellen | 200 `accepted` / 409 `rejected` |
| POST | `/v1/routes/{name}/cancel` | Fahrstraße zurücknehmen | 200 / 409 |
| POST | `/v1/points/{name}/move` | Weiche umstellen, Body `{"position":"left"\|"right"}` | 200 / 409 |
| GET | `/v1/areas` | Stellbereiche mit Grenzen und Erlaubnissen | JSON-Liste |
| POST | `/v1/borders/{Bereich>Nachbar}/request` | Erlaubnis anfordern | 200 / 409 |
| POST | `/v1/borders/{Bereich>Nachbar}/withdraw` | Anforderung zurückziehen | 200 / 409 |
| GET | `/v1/aux` | Offene Hilfshandlungen mit Prüfcode (`check`) und Zählwerk | `{"pending": […], "counter"}` |
| POST | `/v1/aux` | Hilfshandlung anfordern, Body `{"kind","element","position"?}`; ohne Benutzerverwaltung Kopf `X-Railit-Operator` | 200 / 409 / 400 |
| POST | `/v1/aux/{id}/confirm` | Hilfshandlung bestätigen (anderer Bediener), Body `{"kind","element","position"?,"check"}` (wiederholte Hilfshandlung und Prüfcode der Anzeige), `?area=` bei mehreren Bereichen | 200 / 409 / 400 |
| GET | `/v1/ops` | Betriebsführung: Fahrplan, Zuglenkung, Züge mit Zugnummer, Halt, Verspätung | JSON |
| POST | `/v1/ops/ars` | Zuglenkung ein/aus, Body `{"on":true}` | 200 |
| POST | `/v1/ops/trains/{zug}/describe` | Zugnummer eintragen, Body `{"signal"}` | 200 / 400 |
| DELETE | `/v1/ops/trains/{zug}` | Zugnummer löschen | 200 / 400 |
| POST | `/v1/ops/trains/{zug}/hold` bzw. `/release` | Zug zurückhalten bzw. freigeben | 200 / 400 |
| POST | `/v1/ops/trains/{zug}/platform` | Gleiswechsel, Body `{"station","platform"}` | 200 / 400 |
| POST | `/v1/ops/trains/{zug}/delay` | Wartezeit verfügen, Body `{"s":60}` | 200 / 400 |
| GET | `/v1/whoami` | Angemeldeter Benutzer mit Rollen | JSON |
| GET | `/v1/audit?last=50` | Auditprotokoll prüfen, letzte Einträge | `{"records","head","last"}` |
| GET | `/v1/events` | Ereignisstrom (Server-Sent Events, je Ereignis ein JSON-Objekt) | `text/event-stream` |
| GET | `/v1/sim/trains` | Züge der Simulation | JSON-Liste |
| POST | `/v1/sim/trains` | Zug einsetzen, Body `{"name","signal","length_m","speed_kmh","ignore_signals"?,"etcs"?}`; mit `"etcs":true` fährt der Zug nach der Fahrerlaubnis des RBC statt nach Signalbegriffen | 200 / 400 |
| DELETE | `/v1/sim/trains/{name}` | Zug entfernen | 200 / 400 |
| POST | `/v1/sim/faults` | Störung setzen/aufheben, Body `{"kind","element","active"?}` | 200 / 404 |
| GET | `/v1/etcs` | ETCS-Züge: `train`, `mode` (`stand_by`, `full_supervision`, `trip`), `authority` (siehe unten), `end`, `eoa_mm`, `danger_point_mm` | JSON-Liste |
| GET | `/v1/safety` | Je Stellbereich: `area`, `redundancy` (`single`, `two_out_of_two`, `two_out_of_three`), `channels` (`channel`, `active`), `trip` (Grund der Abschaltung oder `null`) | JSON-Liste |
| POST | `/v1/safety/faults` | Fehler in einen Kanal einbringen (Prüfung), Body `{"area"?,"channel","fault"}` mit `fault` aus `drop_inputs`, `stray_command`, `stall` | 200 / 400 |
| POST | `/v1/safety/reintegrate` | Ausgeschlossenen Kanal wieder aufnehmen, Body `{"area"?,"channel"}` | 200 / 400 |
| POST | `/v1/safety/reset` | Sicheren Zustand zurücksetzen (Neubeginn wie nach einem Neustart), Body `{"area"?}` | 200 / 400 |
| GET | `/v1/field/links` | Verbindungen zu den Objektsteuerungen (nur mit `--field eulynx`) | JSON-Liste |
| POST | `/v1/field/links/{Objektsteuerung}` | Übertragungsstrecke stören, Body `{"down"?,"loss_percent"?,"corrupt_percent"?,"delay_ms"?}`; `{}` hebt alle Störungen auf | 200 / 400 |

Namen mit `>` (Grenzen, Fahrstraßen über Grenzen wie `AN1->Strecke`) werden im Pfad kodiert
(`AN1-%3EStrecke`). Bei mehreren Stellbereichen tragen Antworten und Ereignisse das Feld
`area`.

Unbekannte Elemente ergeben `404`, ungültige Eingaben `400`, eine fehlende Anmeldung `401`,
fehlende Rechte `403`.
Fehler haben die Form `{"error": "…"}`.

## Antworten des Kerns

Bedienanforderungen werden im nächsten Zyklus vom Kern entschieden. Die Antwort kommt
synchron:

```json
{"type": "accepted", "request": 1}
{"type": "rejected", "request": 2, "reason": "conflict", "element": null}
```

Ablehnungsgründe: `conflict`, `tvd_not_clear`, `point_locked`, `point_not_operable`,
`route_active`, `route_not_active`, `train_in_route`, `safe_state`, `unknown_element`,
`signal_held`, `aux_busy`, `aux_unknown`, `aux_expired`, `same_operator`,
`aux_mismatch` (Bestätigung weicht ab, Hilfshandlung verfallen), `check_mismatch` (Prüfcode
der Anzeige weicht ab, vom Dienst), `aux_not_applicable`, `no_permission` (Ausfahrt über eine
Grenze ohne Erlaubnis).

## Hilfshandlungen

Hilfshandlungen heben Schutzmechanismen gezielt auf. Ein Bediener fordert sie an, ein
**anderer** bestätigt sie innerhalb der Frist (Vorgabe 30 s); erst dann führt der Kern sie aus
und erhöht das Zählwerk. Der Kopf `X-Railit-Operator: <Nummer>` identifiziert den Bediener.

| `kind` | `element` | Voraussetzung |
|--------|-----------|---------------|
| `release_route` | Fahrstraße | eingestellt (auch befahren) |
| `reset_tvd` | Abschnitt | gestört gemeldet und unverschlossen |
| `substitute` | Fahrstraße | festgelegt oder haltgefallen, Weichen in Lage, Schutzsignale Halt, BÜ geschlossen |
| `force_point` | Weiche, dazu `"position"` | unverschlossen, Abschnitt gestört |
| `take_permission` | Grenze `Bereich>Nachbar` | Erlaubnis zurückholen: weder dieser Kern noch der Nachbar hält sie |

```sh
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" -H "X-Railit-Operator: 1" \
     -H "Content-Type: application/json" \
     -d '{"kind":"release_route","element":"A-N1"}' http://127.0.0.1:7878/v1/aux
# {"type":"accepted","request":7,"aux":{"type":"aux_pending","aux":0,"op":{…},"operator":1}}
curl -s -H "Authorization: Bearer $RAILIT_TOKEN" http://127.0.0.1:7878/v1/aux
# {"pending":[{"area":"Beispielstadt","aux":0,"op":{"kind":"release_route","element":"A-N1"},
#              "operator":1,"since_ms":…,"check":"63CC78"}],"counter":0}
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" -H "X-Railit-Operator: 2" \
     -H "Content-Type: application/json" \
     -d '{"kind":"release_route","element":"A-N1","check":"63CC78"}' \
     http://127.0.0.1:7878/v1/aux/0/confirm
# {"type":"accepted","request":8,"aux":{"type":"aux_executed","aux":0,"op":{…},"counter":1}}
```

**Sichere Anzeige- und Bestätigungskette** (REQ-UI-003, REQ-IXL-022): Die Bestätigung
wiederholt die Hilfshandlung so, wie der zweite Bediener sie liest und eingibt, und trägt den
Prüfcode, den sein Bedienplatz aus der Anzeige berechnet hat. Der Prüfcode sind die ersten
drei Bytes von SHA-256 über

```text
railit-aux-v1\n{Stellbereich}\n{Nummer}\n{kind}\n{element}\n{position oder leer}
```

als sechs Hexziffern in Großbuchstaben. Weicht er vom Code der offenen Hilfshandlung ab,
weist der Dienst die Bestätigung mit `check_mismatch` ab (409, nichts geändert: Die Anzeige
ist gestört). Weicht die wiederholte Hilfshandlung von der offenen ab, lehnt der Kern mit
`aux_mismatch` ab; die Hilfshandlung **verfällt** und muss neu angefordert werden.
`railit aux confirm 0 release-route A-N1 --operator 2` berechnet den Code aus der Anzeige
von `/v1/aux`.

## Bedienplatz

`railitd` liefert unter `/ui/` einen Bedienplatz aus (TypeScript mit
[bQuery](https://www.npmjs.com/package/@bquery/bquery), gebaut mit Bun aus `apps/railitd/ui`;
das Bundle ist eingebettet). Er ist ein gewöhnlicher Client dieser API: `/v1/layout` einmal,
`/v1/state` alle 300 ms, `/v1/events` für Meldungen, Bedienungen über die Pfade oben.
Mit Benutzerdatei oder Token fragt er nach dem Zugangstoken (nur in der Sitzung gespeichert).

- **Fahrstraße:** Start- und Zielsignal anklicken. **Rechtsklick** auf Signal, Weiche,
  Abschnitt oder Grenze: Rücknahme, Umstellen, Erlaubnis, Hilfshandlungen.
- **Hilfshandlungen:** Liste mit „Bestätigen …“; der Dialog zeigt Hilfshandlung, Bediener und
  Prüfcode, das Element ist selbst einzugeben.
- **Anzeigeüberwachung** (REQ-UI-002): Kommt 2 s lang kein Zustandsbild mit fortgeschrittener
  Zeit, wird der Spurplan abgedeckt („ANZEIGE GESTÖRT“) und jede Bedienung gesperrt.
- **Sicherheitsköpfe:** `Content-Security-Policy` ohne Inline-Skripte und fremde Quellen,
  `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Cache-Control: no-store`.

## Sicherer Rechner

Mit `railitd --node` rechnet der Kern eines Stellbereichs auf einem Knoten (`railit-node`,
RaSTA über UDP; `--node lockstep` im Dienst). Das Zustandsbild zeigt die Anbindung unter
`areas[].safety.node`:

```json
{"link": "udp://127.0.0.1:7979", "up": true, "available": true, "synced": true,
 "cycles": 1834, "syncs": 1, "divergences": 0, "link_losses": 0, "protocol_errors": 0,
 "last_problem": null}
```

`available = false`: Der Knoten liefert keine Zyklen, alle Signale des Bereichs stehen auf
Halt (Rückfallebene). `synced = false`: Der Spiegel wird gerade abgeglichen; der Zustand des
Bereichs ist bis dahin nicht verlässlich. Redundanz und Kanäle unter `safety` sind die des
Knotens; Handlungen an der Sicherheitsschicht (`/v1/safety/…`) gehen an ihn.

## Zustandsbild

`/v1/state` enthält `network`, `time_ms`, `cycle`, `safe_state` sowie:

| Feld | Inhalt je Eintrag |
|------|-------------------|
| `signals` | `name`, `commanded` (`stop`/`proceed`), `report`, `substitute` (Zs1), `picture` (z. B. `"Ks1 Zs3(4)"`, `"Hp2 Vr0"`), `aspects` (strukturiert), `flank_holds` |
| `points` | `name`, `report`, `commanded`, `locked_by` (Fahrweg), `locked_position`, `holds` (Durchrutschweg/Flankenschutz) |
| `tvds` | `name`, `occupancy`, `locked_by` (Fahrweg), `overlap_of` (Durchrutschweg) |
| `routes` | `name`, `start`, `end`, `phase` |
| `level_crossings` | `name`, `report` (`open`, `closing`, `closed`, `disturbed`), `close_commanded`, `holds` |
| `pending_aux` | `aux`, `op`, `operator`, `since_ms` |
| `aux_counter` | Zählwerk |

## Ereignisse

Jedes Ereignis trägt `time_ms` (Simulationszeit) und `type`:

| `type` | Felder | Bedeutung |
|--------|--------|-----------|
| `accepted` / `rejected` | `request`, `reason`, `element` | Antwort auf eine Anforderung |
| `route_phase` | `route`, `phase` | Phasenwechsel: `idle`, `setting`, `locked`, `proceed`, `occupied`, `arrived` (Ziel erreicht, Durchrutschweg noch festgelegt), `cancelling`, `halted` |
| `alarm` | `alarm`, `element` | `setting_timeout`, `signal_dropped`, `train_entered_cancelled_route`, `signal_defective`, `signal_mismatch`, `unknown_element`, `unauthorized_occupancy`, `signal_passed_at_danger`, `border_lost` (keine aktuelle Meldung des Nachbarn), `permission_conflict` |
| `permission` | `border`, `holding` | Erlaubnis an einer Grenze übernommen bzw. abgegeben |
| `aux_pending` / `aux_executed` / `aux_expired` | `aux`, `op`, `operator` bzw. `counter` | Hilfshandlung angefordert, ausgeführt, verfallen |
| `train_described`, `train_moved`, `train_left`, `train_erased`, `arrived`, `departed`, `route_requested`, `permission_requested`, `spawned`, `spawn_failed` | `train`, … (`"source":"ops"`) | Betriebsführung: Zugnummern, Halte mit `delay_s`, Anforderungen der Zuglenkung |
| `train_registered`, `train_deregistered`, `authority_granted`, `authority_shortened`, `emergency_stop` | `train`, `end`, `eoa_mm` bzw. `reason` (`"source":"etcs"`) | RBC: Anmeldung, Fahrerlaubnis, Verkürzung, Notbremsung |
| `safety` | `event` (`channel_excluded` mit `channel`, `divergence`; `tripped` mit `trip`), `area` | Sicherheitsschicht: Kanal ausgeschlossen, Abschaltung |
| `sim` | `event`, `train`, … | Simulation: `train_stopped`, `train_started`, `train_left`, `train_arrived`, `train_derailed`, `point_trailed`, `signal_passed_at_danger`, `substitute_passed`, `level_crossing_not_closed` |

## Störungsarten der Simulation

`point_disturbed`, `tvd_disturbed`, `tvd_occupied`, `signal_defective`, `lx_disturbed`.

## Netze mit Stellbereichen

`--network` nimmt rail-it-TOML oder railML 3 (Endung `.xml` bzw. `.railml`).
Hat das Netz Stellbereiche, ist `--plan` ein Verzeichnis mit `<Bereich>.rplan`
(`railit plan compile netz.toml --out plaene/`) und `--journal` ein Verzeichnis mit
`<Bereich>.journal`. Mit `--timetable fahrplan.toml` setzt die Betriebsführung Züge nach Plan
ein, mit `--ars` lenkt sie sie.

## ETCS Level 2

Das RBC leitet jede Fahrerlaubnis aus den Fahrstraßen mit Fahrt ab (REQ-ETCS-002). Ein
Abschnitt in `authority.sections` hat `area`, `route` (`null` für den Annäherungsabschnitt
hinter einer Bereichsgrenze), `length_mm` und `speed_kmh` (falls beschränkt). `end` ist das
Zielsignal, `"Netzgrenze"`, `"Prellbock"` oder eine Grenze; `eoa_mm` und `danger_point_mm`
sind Abstände ab dem Bezugspunkt. `railit etcs` zeigt dasselbe auf der Kommandozeile.

## Feldanbindung über EULYNX und RaSTA

Mit `railitd --field eulynx` laufen alle Stellbefehle und Meldungen als EULYNX-SCI-Telegramme
über je eine RaSTA-Verbindung zu den simulierten Objektsteuerungen `OC-Weichen` (SCI-P),
`OC-Signale` (SCI-LS), `OC-Gleisfrei` (SCI-TDS) und `OC-BUe` (SCI-LX). `/v1/field/links`
zeigt je Verbindung `controller`, `interface`, `elements`, `up`, `discarded` (verworfene
PDUs), `bad_telegrams` und die eingestellten `faults`. Fällt eine Verbindung aus, behandelt
der Kern ihre Elemente nach der Meldezeit als gestört (REQ-COM-006).

## Redundante Kanäle

Mit `railitd --channels 2` (2-von-2) bzw. `--channels 3` (2-von-3) rechnet jeder Stellbereich
in so vielen Kanälen; der Voter gibt nur übereinstimmende Ausgaben weiter. `--watchdog-ms N`
überwacht die Rechenzeit je Zyklus. Ist die Sicherheitsschicht abgeschaltet, zeigt
`/v1/state` `safe_state`, alle Signale stehen auf Halt und Anforderungen werden mit `409`
abgelehnt, bis `/v1/safety/reset` den Bereich neu beginnen lässt. `railit safety` zeigt den
Zustand auf der Kommandozeile.

## Journal

Mit `railitd --journal DATEI [--checkpoint-every N]` schreibt der Dienst ein Betriebsjournal
(JSON Lines, Hashkette). `railit journal verify DATEI` prüft die Kette,
`railit journal replay DATEI --plan PLAN.rplan` spielt es ab und vergleicht Ausgaben und
Zustand. Alle zehn Minuten Betrieb steht ein Zustandsabzug im Journal; das Replay spielt die
Abschnitte dazwischen parallel ab. `railit journal state DATEI --plan PLAN.rplan --at SEK` zeigt
den Zustand zu einem Zeitpunkt, `railit journal events … --from SEK --to SEK` die Stellbefehle
und Ereignisse eines Zeitfensters; dieselbe Rückschau liefert der Dienst unter
`/v1/journal/state` und `/v1/journal/events` (Zeitpunkte in Millisekunden seit Beginn des
Journals). `railit journal follow DATEI --plan PLAN.rplan` liest ein wachsendes Journal mit und
rechnet jede neue Zeile sofort nach. Mit `--node` schreibt der Spiegel das Journal: geprüfte
Zyklen des Knotens und jeden Zustandsabgleich als Übernahme.

## Metriken

`/v1/metrics` liefert Metriken im Prometheus-Textformat (Version 0.0.4), mit derselben Anmeldung
wie `/v1/state` (Prometheus: `authorization: { credentials: <Token> }`).

| Metrik | Art | Inhalt |
|--------|-----|--------|
| `railit_cycles_total`, `railit_cycle_overruns_total` | Zähler | Zyklen des Dienstes; Zyklen länger als die Zykluszeit |
| `railit_cycle_seconds` | Histogramm | Rechenzeit eines Zyklus (alle Bereiche, Feld, Betriebsführung) |
| `railit_cycle_seconds_max` | Wert | Längster Zyklus seit dem Start |
| `railit_requests_total{result,reason}` | Zähler | Angenommene und abgelehnte Anforderungen nach Grund |
| `railit_alarms_total{alarm}` | Zähler | Störungs- und Gefahrenmeldungen der Kerne |
| `railit_aux_executed_total`, `railit_aux_counter{area}` | Zähler, Wert | Hilfshandlungen seit Start bzw. Zählwerk je Bereich |
| `railit_channel_exclusions_total`, `railit_safety_trips_total` | Zähler | Ausschlüsse von Kanälen, Abschaltungen der Sicherheitsschicht |
| `railit_safe_state{area}`, `railit_channels_active{area}` | Wert | Sicherer Zustand, mitwählende Kanäle |
| `railit_routes{area,phase}` | Wert | Eingestellte Fahrstraßen je Phase |
| `railit_journal_ok` | Wert | 0, sobald ein Journal abgebrochen ist |
| `railit_etcs_emergency_stops_total` | Zähler | ETCS-Notbremsungen |
| `railit_node_up`, `…_available`, `…_synced{area}` | Wert | Anbindung des Knotens (nur mit `--node`) |
| `railit_node_cycles_total`, `…_syncs_total`, `…_divergences_total`, `…_link_losses_total`, `…_protocol_errors_total` | Zähler | Knoten: Zyklen, Abgleiche, Abweichungen, Verbindungsabbrüche, Protokollfehler |

Metriken sind reine Beobachtung (SIL 0) ohne Rückwirkung auf den Betrieb.

## Beispiel

```sh
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" \
     http://127.0.0.1:7878/v1/routes/A-N1/set
curl -sN -H "Authorization: Bearer $RAILIT_TOKEN" http://127.0.0.1:7878/v1/events
```

## gRPC

Vertrag: [`proto/railit/v1/railit.proto`](../proto/railit/v1/railit.proto). Dienste:

| Dienst | Methoden |
|--------|----------|
| `railit.v1.Interlocking` | `GetState`, `SetRoute`, `CancelRoute`, `MovePoint`, `RequestPermission`, `WithdrawPermission`, `RequestAux`, `ConfirmAux` |
| `railit.v1.Operations` | `GetOperations`, `SetArs`, `Describe`, `Erase`, `Hold`, `Release`, `SetPlatform`, `Delay` |
| `railit.v1.Simulation` | `ListTrains`, `AddTrain`, `RemoveTrain`, `SetFault` |
| `railit.v1.Events` | `Subscribe` (Server-Streaming, optional nach `type` gefiltert) |
| `railit.v1.Etcs` | `GetTrains` |
| `railit.v1.Safety` | `GetSafety`, `InjectFault`, `Reintegrate`, `Reset` |
| `railit.v1.Diagnostics` | `WhoAmI`, `GetPlan`, `GetAudit` |
| `railit.v1.Infrastructure` | `GetLayout` (Spurplan-Geometrie als JSON) |

gRPC läuft über HTTP/2 auf dem Port der REST-API (ohne TLS als h2c, mit TLS per ALPN).
Anmeldung als Metadaten `authorization: Bearer <token>` oder per Client-Zertifikat. Eine
Ablehnung durch den Kern ist eine Antwort (`Answer.accepted = false`, `reason`), kein Fehler;
Fehlercodes: `NOT_FOUND`, `INVALID_ARGUMENT`, `PERMISSION_DENIED`, `UNAUTHENTICATED`,
`UNAVAILABLE`, `DEADLINE_EXCEEDED`.

Die Kommandozeile `railit` deckt alle Pfade ab (`railit state`, `railit route set A-N1`,
`railit aux request release-route A-N1 --operator 1`, `railit watch`, …).
