# Betriebsführung und Fahrplan

Die Betriebsführung (`railit-ops`) arbeitet **über** den Kernen: Sie verfolgt Zugnummern,
vergleicht mit dem Fahrplan und stellt Anforderungen – genau wie ein Bediener. Sie kann nichts
erzwingen; jede Anforderung prüft der Kern (SIL 0).

## Zugnummernmeldung

Zugnummern stehen an Signalen und wandern mit den Belegungen weiter. Neue Züge erhalten ihre
Nummer am Einfahrsignal; von Hand:

```sh
railit ops describe RE7 A      # Zugnummer RE7 an Signal A eintragen
railit ops erase RE7
```

## Fahrplan

Ein Fahrplan nennt je Zug Einfahrt, Halte und Zeiten ab Betriebsbeginn
([Format](/referenz/fahrplan)):

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
```

Betriebsstellen und Bahnsteiggleise stehen im Netz (`[[station]]`). In der Simulation setzt
`enter` den Zug zur angegebenen Zeit vor dem Signal ein.

## Zuglenkung

```sh
railitd --network nordbahn.toml --plan nordbahn/ --trusted-key railit.pub \
        --timetable nordbahn-fahrplan.toml --ars --speed 10
railit ops ars on|off
```

Die Zuglenkung fordert für Züge mit Fahrplan die nächsten Fahrstraßen und Erlaubnisse an, hält
an den Bahnsteigen bis zur Abfahrtszeit und stellt Selbstblocksignale (`auto = true`)
selbsttätig. Über die Reihenfolge entscheiden die Kerne.

## Disposition

```sh
railit ops list                     # Züge mit Zugnummer, nächstem Halt, Verspätung
railit ops hold RE1                 # zurückhalten
railit ops release RE1              # freigeben
railit ops platform RE1 Bstadt 1    # Gleiswechsel
railit ops delay RE1 120            # Wartezeit verfügen (Sekunden)
```

Am Bedienplatz stehen Zuglenkung, Zurückhalten und Freigeben im Feld „Züge“.
