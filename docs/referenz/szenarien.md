# Szenarien

Ein Szenario ist eine TOML-Datei aus einem Kopf `[scenario]` und Schritten `[[step]]`. Es läuft
deterministisch mit Kern und Simulator in simulierter Zeit. Einführung und Beispiele:
[Szenarien](../leitfaden/szenarien); über zwanzig Szenarien liegen unter `tests/scenarios/`.

```sh
railit scenario run tests/scenarios/*.toml [--channels 2|3] [--node] [--journal] [--ils]
```

## `[scenario]`

| Schlüssel | Typ | Vorgabe | Bedeutung |
|-----------|-----|---------|-----------|
| `name` | Text | | Name |
| `description` | Text | | Beschreibung |
| `network` | Pfad | | Netz (TOML oder railML), relativ zur Szenariodatei |
| `cycle_ms` | Zahl | 100 | Zykluszeit |
| `point_move_time_ms` | Zahl | Simulator | Umlaufzeit der Weichen |
| `lx_close_time_ms` | Zahl | Simulator | Schließzeit der Bahnübergänge |
| `points` | Tabelle | | Anfangslagen: `{ W1 = "left" }` |
| `rules` | Tabelle | Lehrregelwerk | Zeiten des Regelwerks, siehe unten |
| `timetable` | Pfad | | Fahrplan, relativ zur Szenariodatei; schaltet die Betriebsführung ein |
| `ars` | bool | `false` | Zuglenkung |
| `ops` | bool | `false` | Nur Zugnummernmeldung und Selbstblock |
| `field` | `direct` · `eulynx` | `direct` | Stellbefehle und Meldungen über EULYNX SCI und RaSTA zu Objektsteuerungen |
| `channels` | 1 · 2 · 3 | 1 | Kanäle je Stellbereich (2-von-2, 2-von-3) |
| `node` | bool | `false` | Jeder Bereich auf einem Knoten des sicheren Rechners |
| `borders` | `direct` · `ils` | `direct` | Grenzmeldungen zwischen den Stellbereichen über EULYNX SCI-ILS und RaSTA |

`--channels` und `--node` auf der Kommandozeile gelten für Szenarien ohne eigene Angabe;
`--journal` schreibt jeden Bereich mit und spielt das Journal am Ende ab; `--ils` überträgt
die Grenzmeldungen aller Szenarien über SCI-ILS.

### `rules`

Alle Werte in Millisekunden: `point_move_timeout_ms`, `report_timeout_ms`,
`cancel_time_lock_ms`, `overlap_release_time_ms`, `aux_confirm_timeout_ms`,
`substitute_duration_ms`, `border_settle_time_ms`.

## `[[step]]`

Ein Schritt führt seine Aktionen aus, läuft dann

- eine feste Zeit (`wait_s`) oder
- bis `until` gilt, höchstens `timeout_s` (Vorgabe 300 s; sonst schlägt das Szenario fehl),

und prüft danach `expect`. `note` beschreibt den Schritt in der Ausgabe.

### Aktionen des Stellwerks

| Aktion | Wert | Wirkung |
|--------|------|---------|
| `set_route` | Fahrstraße | Fahrstraße anfordern |
| `cancel_route` | Fahrstraße | Fahrstraße zurücknehmen |
| `move_point` | `{ point, position }` | Einzelne Weiche umstellen (`left`/`right`) |
| `request_permission` | Grenze | Erlaubnis an einer Bereichsgrenze anfordern (z. B. `"Adorf>Strecke"`) |
| `aux` | `{ kind, element, position, operator }` | Hilfshandlung anfordern |
| `confirm_aux` | `{ operator }` bzw. `{ operator, kind, element, position }` | Zuletzt angeforderte Hilfshandlung bestätigen; mit Angaben nennt der zweite Bediener sie selbst |

Hilfshandlungen (`kind`): `release_route` (Fahrstraße), `reset_tvd` (Abschnitt), `substitute`
(Ersatzsignal, Fahrstraße), `force_point` (Weiche mit `position`), `take_permission`
(Grenze). Weicht eine selbst genannte Bestätigung ab, lehnt der Kern mit `aux_mismatch` ab und
die Hilfshandlung verfällt.

### Simulation

| Aktion | Wert | Wirkung |
|--------|------|---------|
| `add_train` | `{ name, signal, length_m, speed_kmh, ignore_signals, etcs, pzb }` | Zug vor einem Signal einsetzen; `ignore_signals` überfährt Halt (Prüfung des Schutzes); `pzb` bremst ihn dann an wirksamen Gleismagneten zwangsweise (REQ-MOV-007) |
| `fault` / `clear_fault` | `{ kind, element }` | Störung setzen bzw. aufheben |
| `link` | `{ controller, down, loss_percent, corrupt_percent, delay_ms }` | Übertragungsstrecke zu einer Objektsteuerung stören (nur `field = "eulynx"`) |
| `ils_link` | `{ link, down, loss_percent, corrupt_percent, delay_ms }` | SCI-ILS-Strecke zwischen zwei Stellwerken stören, z. B. `link = "Adorf-Strecke"` (nur `borders = "ils"`) |

Störungen (`kind`): `point_disturbed`, `tvd_disturbed`, `tvd_occupied`, `signal_defective`,
`lx_disturbed`.

### Betriebsführung

| Aktion | Wert | Wirkung |
|--------|------|---------|
| `describe` | `{ train, signal }` | Zugnummer an einem Signal eintragen |
| `erase` | Zugnummer | Zugnummer löschen |
| `hold` / `release` | Zugnummer | Zug zurückhalten bzw. freigeben |
| `platform` | `{ train, station, platform }` | Bahnsteiggleis ändern |
| `delay` | `{ train, s }` | Verspätung in Sekunden |
| `set_ars` | bool | Zuglenkung ein- oder ausschalten |

Mit eingeschalteter Betriebsführung trägt `add_train` die Zugnummer am Signal ein.

### Redundanz

| Aktion | Wert | Wirkung |
|--------|------|---------|
| `channel_fault` | `{ channel, fault, area }` | Kanal stören: `drop_inputs`, `stray_command`, `stall` |
| `reintegrate` | `{ channel, area }` | Ausgeschlossenen Kanal wieder aufnehmen |
| `reset_safety` | `true` oder Bereich | Sicherheitsschicht aller bzw. eines Bereichs zurücksetzen |

`area` ist optional; ohne Angabe gilt der erste bzw. einzige Bereich.

## Bedingungen (`until`, `expect`)

| Bedingung | Form | Werte |
|-----------|------|-------|
| `route` | `{ Fahrstraße = Phase }` | `idle`, `setting`, `locked`, `proceed`, `occupied`, `arrived`, `cancelling` |
| `signal` | `{ Signal = Begriff }` | `stop`, `proceed` (befohlener Begriff) |
| `picture` | `{ Signal = Bild }` | Signalbild, z. B. `"Ks1 Zs3(4)"` |
| `substitute` | `{ Signal = bool }` | Ersatzsignal an |
| `point` | `{ Weiche = Meldung }` | `left`, `right`, `no_end_position`, `disturbed` |
| `tvd` | `{ Abschnitt = Meldung }` | `clear`, `occupied`, `disturbed` |
| `lx` | `{ BÜ = Meldung }` | `open`, `closing`, `closed`, `disturbed` |
| `train` | `{ Zug = Zustand }` | `running`, `waiting`, `arrived`, `left`, `derailed`, `tripped`, `braked` (nach PZB-Zwangsbremsung) |
| `permission` | `{ Grenze = bool }` | Erlaubnis gehalten |
| `accepted` | bool | Letzte Anforderung angenommen |
| `rejected` | Grund | Letzte Anforderung abgelehnt, mit Grund ([API](../api#antworten-des-kerns)) |
| `alarm`, `no_alarm` | Liste | Meldungen der Kerne vorhanden bzw. nicht vorhanden |
| `sim_event` | Liste | Ereignisse des Simulators, z. B. `signal_passed_at_danger`, `point_trailed`, `crossing_collision`, `pzb_brake`, `pzb_stopped` |
| `aux_counter` | Zahl | Stand des Zählwerks für Hilfshandlungen |
| `safe_state` | bool | Kern im sicheren Zustand |
| `berth` | `{ Zugnummer = Signal }` | Zugnummer steht am Signal; `"left"` nach Verlassen des Netzes |
| `at_stop` | `{ Zugnummer = "Betriebsstelle/Gleis" }` | Zug hält am Bahnsteig |
| `ops_event` | Liste | Ereignisarten der Betriebsführung |
| `etcs` | `{ Zug = Betriebsart }` | `stand_by`, `full_supervision`, `trip` |
| `etcs_end` | `{ Zug = Ende }` | Ende der Fahrerlaubnis, Signalname oder `"Netzgrenze"` |
| `etcs_event` | Liste | Ereignisarten des RBC |
| `link_up` | `{ Objektsteuerung = bool }` | Verbindung steht |
| `ils_up` | `{ Verbindung = bool }` | SCI-ILS-Verbindung zwischen zwei Stellwerken steht, z. B. `{ "Adorf-Strecke" = true }` |
| `safety_trip` | bool | Eine Sicherheitsschicht hat abgeschaltet |
| `safety_event` | Liste | `channel_excluded`, `tripped` |
| `active_channels` | Liste von bool | Mitwählende Kanäle, z. B. `[true, true, false]` |
