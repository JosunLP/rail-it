# API von `railitd` (v1)

`railitd` stellt eine REST/JSON-API und einen Ereignisstrom bereit. Standardadresse:
`http://127.0.0.1:7878`. Ist ein Token gesetzt (`--token` oder `RAILIT_TOKEN`), verlangen
alle Pfade außer `/v1/health` den Kopf `Authorization: Bearer <token>`.

| Methode | Pfad | Zweck | Antwort |
|---------|------|-------|---------|
| GET | `/v1/health` | Lebenszeichen | `{"status":"ok"\|"safe_state", "network", "cycle", "time_ms", "safe_state"}` |
| GET | `/v1/plan` | Plan: Prüfsumme, Signatur, Elemente, Fahrstraßen mit Weichen, Abschnitten, Feindlichkeiten | JSON |
| GET | `/v1/state` | Zustandsbild: Signale, Weichen, Abschnitte, Fahrstraßen | JSON |
| POST | `/v1/routes/{name}/set` | Fahrstraße einstellen | 200 `accepted` / 409 `rejected` |
| POST | `/v1/routes/{name}/cancel` | Fahrstraße zurücknehmen | 200 / 409 |
| POST | `/v1/points/{name}/move` | Weiche umstellen, Body `{"position":"left"\|"right"}` | 200 / 409 |
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
`route_active`, `route_not_active`, `train_in_route`, `safe_state`, `unknown_element`.

## Ereignisse

Jedes Ereignis trägt `time_ms` (Simulationszeit) und `type`:

| `type` | Felder | Bedeutung |
|--------|--------|-----------|
| `accepted` / `rejected` | `request`, `reason`, `element` | Antwort auf eine Anforderung |
| `route_phase` | `route`, `phase` | Phasenwechsel: `idle`, `setting`, `locked`, `proceed`, `occupied`, `cancelling`, `halted` |
| `alarm` | `alarm`, `element` | `setting_timeout`, `signal_dropped`, `train_entered_cancelled_route`, `signal_defective`, `signal_mismatch`, `unknown_element` |
| `sim` | `event`, `train`, … | Simulation: `train_stopped`, `train_started`, `train_left`, `train_arrived`, `train_derailed`, `point_trailed`, `signal_passed_at_danger` |

## Störungsarten der Simulation

`point_disturbed`, `tvd_disturbed`, `tvd_occupied`, `signal_defective`.

## Beispiel

```sh
curl -s -X POST -H "Authorization: Bearer $RAILIT_TOKEN" \
     http://127.0.0.1:7878/v1/routes/A-N1/set
curl -sN -H "Authorization: Bearer $RAILIT_TOKEN" http://127.0.0.1:7878/v1/events
```

Die Kommandozeile `railit` deckt alle Pfade ab (`railit state`, `railit route set A-N1`,
`railit watch`, …).
