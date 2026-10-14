# Fahrstraßen und Hilfshandlungen

## Lebenszyklus einer Fahrstraße

| Phase | Bedeutung |
|-------|-----------|
| `idle` | nicht eingestellt |
| `setting` | Elemente reserviert, Weichen laufen um |
| `locked` | festgelegt: Weichen überwacht in Lage, alles verschlossen |
| `proceed` | Startsignal zeigt Fahrt |
| `occupied` | Zug in der Fahrstraße; Abschnitte lösen hinter ihm auf |
| `arrived` | Zug am Ziel; der Durchrutschweg löst nach seiner Frist auf |
| `cancelling` | zurückgenommen, Zeitverschluss läuft |
| `halted` | Signal ist wegen einer Störung auf Halt gefallen – nur Rücknahme möglich |

Fahrt gibt der Kern nur, wenn alle Abschnitte frei, alle Weichen (auch Durchrutschweg und
Flankenschutz) überwacht in Lage und alle Bahnübergänge geschlossen sind – und prüft das in
jedem Zyklus erneut.

```sh
railit route list
railit route set A-N1
railit route cancel A-N1        # nach Fahrt mit Zeitverschluss
railit point move W1 left       # nur unverschlossen und in freiem Abschnitt
```

## Ablehnungsgründe

| Grund | Bedeutung |
|-------|-----------|
| `conflict` | feindliche Fahrstraße eingestellt oder Element anderweitig verschlossen |
| `tvd_not_clear` | Abschnitt besetzt oder gestört |
| `point_locked`, `point_not_operable` | Weiche verschlossen bzw. nicht stellbar (Abschnitt besetzt, Lage gestört) |
| `route_active`, `route_not_active` | Fahrstraße ist schon eingestellt bzw. nicht eingestellt |
| `train_in_route` | ein Zug befährt die Fahrstraße |
| `signal_held` | Signal als Flankenschutz in Halt gehalten |
| `no_permission` | Fahrt über eine Grenze ohne Erlaubnis |
| `safe_state` | Kern im sicheren Zustand |
| `same_operator` | dieselbe Person darf eine Hilfshandlung nicht bestätigen |
| `aux_mismatch` | Bestätigung wiederholt die Hilfshandlung nicht – sie verfällt |
| `aux_expired`, `aux_unknown`, `aux_busy`, `aux_not_applicable` | Hilfshandlung verfallen, unbekannt, zu viele offen bzw. hier nicht anwendbar |
| `unknown_element` | Element gibt es im Plan nicht |

## Hilfshandlungen

| Art | Wirkung |
|-----|---------|
| `release-route` | Fahrstraßenhilfsauflösung: Fahrstraße sofort auflösen |
| `reset-tvd` | Achszählgrundstellung eines Abschnitts |
| `substitute` | Ersatzsignal (Zs1) für eine festgelegte Fahrstraße |
| `force-point` | Weiche im gestörten Abschnitt umstellen (mit `--position left\|right`) |

```sh
railit aux request reset-tvd T2 --operator 1
railit aux list                                   # offen, mit Prüfcode
railit aux confirm 3 reset-tvd T2 --operator 2    # anderer Bediener, Hilfshandlung wiederholt
```

Jede Hilfshandlung verfällt, wenn sie nicht rechtzeitig bestätigt wird; das Zählwerk zählt die
ausgeführten.

## Sicherer Zustand

Verletzt ein Zustand eine Invariante (etwa zwei Eigentümer eines Abschnitts), geht der Kern in
den **sicheren Zustand**: alle Signale Halt, keine Weichen mehr stellen, alle Anforderungen
abgelehnt. Zurück nur durch die Instandhaltung (`railit safety reset`), danach gelten alle
Elemente als unbekannt.
