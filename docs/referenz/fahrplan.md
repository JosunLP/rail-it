# Fahrplan

Ein Fahrplan beschreibt Züge mit ihren Halten und – für die Simulation – wann und wo sie ins
Netz einfahren. `railitd --timetable plan.toml [--ars]` und Szenarien mit
`timetable = "…"` lesen ihn; die Zuglenkung (`--ars`) fordert danach Fahrstraßen und
Erlaubnisse selbsttätig an. Einführung: [Betriebsführung und Fahrplan](../leitfaden/betriebsfuehrung).

```toml
[timetable]
name = "Nordbahn Grundtakt"

[[train]]
number = "RE1"
enter = { signal = "AA", at = "00:00:10", length_m = 100, speed_kmh = 80 }
stops = [
    { station = "Adorf", platform = "1", arr = "00:00:40", dep = "00:01:30" },
    { station = "Bstadt", platform = "2", arr = "00:06:00" },
]
exit = false
```

Zeiten sind `HH:MM:SS` oder `HH:MM` ab Beginn der Simulation bzw. des Betriebs.

## `[timetable]`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name des Fahrplans (optional) |

## `[[train]]`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `number` | Text | Zugnummer, eindeutig |
| `enter` | Tabelle | Einsetzen in die Simulation (optional) |
| `stops` | Liste | Halte in Fahrtrichtung |
| `exit` | bool, Vorgabe `false` | Nach dem letzten Halt das Netz verlassen statt dort stehen zu bleiben |

### `enter`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `signal` | Text | Der Zug steht mit der Spitze vor diesem Signal |
| `at` | Zeit | Zeitpunkt des Einsetzens |
| `length_m` | Zahl | Zuglänge in Metern |
| `speed_kmh` | Zahl | Geschwindigkeit in km/h |
| `etcs` | bool, Vorgabe `false` | Zug mit ETCS-Fahrzeuggerät (Level 2) |

### `stops`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `station` | Text | Betriebsstelle (`[[station]]` im Netz) |
| `platform` | Text | Bahnsteiggleis der Betriebsstelle |
| `arr` | Zeit | Planmäßige Ankunft (optional) |
| `dep` | Zeit | Planmäßige Abfahrt, frühestens (optional) |
| `dwell_s` | Zahl | Mindesthaltezeit in Sekunden (sonst die Vorgabe des Dienstes) |

## Prüfungen

Beim Laden prüft rail-it den Fahrplan gegen das Netz und meldet alle Abweichungen:

- Zug ohne Nummer oder mit doppelter Nummer,
- unbekanntes Signal in `enter`,
- unbekannte Betriebsstelle oder unbekanntes Bahnsteiggleis,
- Zeiten nicht aufsteigend (vom Einsetzen über alle Ankünfte und Abfahrten),
- Zeiten, die nicht `HH:MM:SS` oder `HH:MM` sind.
