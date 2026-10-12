# API von `railitd` (v1)

`railitd` stellt eine REST/JSON-API und einen Ereignisstrom bereit. Standardadresse:
`http://127.0.0.1:7878`. Ist ein Token gesetzt (`--token` oder `RAILIT_TOKEN`), verlangen
alle Pfade außer `/v1/health` den Kopf `Authorization: Bearer <token>`.

| Methode | Pfad | Zweck | Antwort |
|---------|------|-------|---------|
| GET | `/v1/health` | Lebenszeichen | `{"status":"ok"\|"safe_state", "network", "cycle", "time_ms", "safe_state"}` |
| GET | `/v1/plan` | Plan: Prüfsumme, Signatur, Elemente, Fahrstraßen mit Weichen, Abschnitten, Geschwindigkeit, Durchrutschweg, Flankenschutz, Bahnübergängen, Feindlichkeiten | JSON |
| GET | `/v1/state` | Zustandsbild (siehe unten) | JSON |
| POST | `/v1/routes/{name}/set` | Fahrstraße einstellen | 200 `accepted` / 409 `rejected` |
| POST | `/v1/routes/{name}/cancel` | Fahrstraße zurücknehmen | 200 / 409 |
| POST | `/v1/points/{name}/move` | Weiche umstellen, Body `{"position":"left"\|"right"}` | 200 / 409 |
| GET | `/v1/aux` | Offene Hilfshandlungen und Zählwerk | `{"pending": […], "counter"}` |
| POST | `/v1/aux` | Hilfshandlung anfordern, Body `{"kind","element","position"?}`, Kopf `X-Railit-Operator` | 200 / 409 / 400 |
| POST | `/v1/aux/{id}/confirm` | Hilfshandlung bestätigen, Kopf `X-Railit-Operator` (anderer Bediener) | 200 / 409 / 400 |
| GET | `/v1/events` | Ereignisstrom (Server-Sent Events, je Ereignis ein JSON-Objekt) | `text/event-stream` |
| GET | `/v1/sim/trains` | Züge der Simulation | JSON-Liste |
| POST | `/v1/sim/trains` | Zug einsetzen, Body `{"name","signal","length_m","speed_kmh","ignore_signals"?}` | 200 / 400 |
| DELETE | `/v1/sim/trains/{name}` | Zug entfernen | 200 / 400 |
| POST | `/v1/sim/faults` | Störung setzen/aufheben, Body `{"kind","element","active"?}` | 200 / 404 |

Unbekannte Elemente ergeben `404`, ungültige Eingaben `400`, eine fehlende Anmeldung `401`.
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
`aux_not_applicable`.

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

```sh
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" -H "X-Railit-Operator: 1" \
     -H "Content-Type: application/json" \
     -d '{"kind":"release_route","element":"A-N1"}' http://127.0.0.1:7878/v1/aux
# {"type":"accepted","request":7,"aux":{"type":"aux_pending","aux":0,"op":{…},"operator":1}}
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" -H "X-Railit-Operator: 2" \
     http://127.0.0.1:7878/v1/aux/0/confirm
# {"type":"accepted","request":8,"aux":{"type":"aux_executed","aux":0,"op":{…},"counter":1}}
```

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
| `alarm` | `alarm`, `element` | `setting_timeout`, `signal_dropped`, `train_entered_cancelled_route`, `signal_defective`, `signal_mismatch`, `unknown_element`, `unauthorized_occupancy`, `signal_passed_at_danger` |
| `aux_pending` / `aux_executed` / `aux_expired` | `aux`, `op`, `operator` bzw. `counter` | Hilfshandlung angefordert, ausgeführt, verfallen |
| `sim` | `event`, `train`, … | Simulation: `train_stopped`, `train_started`, `train_left`, `train_arrived`, `train_derailed`, `point_trailed`, `signal_passed_at_danger`, `substitute_passed`, `level_crossing_not_closed` |

## Störungsarten der Simulation

`point_disturbed`, `tvd_disturbed`, `tvd_occupied`, `signal_defective`, `lx_disturbed`.

## Journal

Mit `railitd --journal DATEI [--checkpoint-every N]` schreibt der Dienst ein Betriebsjournal
(JSON Lines, Hashkette). `railit journal verify DATEI` prüft die Kette,
`railit journal replay DATEI --plan PLAN.rplan` spielt es ab und vergleicht Ausgaben und
Zustand.

## Beispiel

```sh
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" \
     http://127.0.0.1:7878/v1/routes/A-N1/set
curl -sN -H "Authorization: Bearer $RAILIT_TOKEN" http://127.0.0.1:7878/v1/events
```

Die Kommandozeile `railit` deckt alle Pfade ab (`railit state`, `railit route set A-N1`,
`railit aux request release-route A-N1 --operator 1`, `railit watch`, …).
