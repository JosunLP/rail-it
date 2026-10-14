# Szenarien

Ein Szenario beschreibt in TOML, was im Betrieb passiert und was danach gelten muss. Es läuft
deterministisch mit Kern und Simulator in simulierter Zeit – ideal für Ausbildung,
Regressionstests und Abnahmen.

```toml
[scenario]
name = "Bahnübergang"
network = "../../examples/beispielstadt.toml"   # relativ zur Szenariodatei
lx_close_time_ms = 8000

[[step]]
note = "Zug steht vor N1"
add_train = { name = "RB3", signal = "N1", length_m = 100, speed_kmh = 60 }
wait_s = 1
expect = { train = { RB3 = "waiting" }, lx = { "BÜ1" = "open" } }

[[step]]
note = "N1-GE einstellen: Bahnübergang schließt, Signal bleibt zunächst Halt"
set_route = "N1-GE"
wait_s = 5
expect = { accepted = true, lx = { "BÜ1" = "closing" }, signal = { N1 = "stop" } }

[[step]]
note = "Bahnübergang geschlossen: Fahrt"
until = { signal = { N1 = "proceed" } }
timeout_s = 10
```

Jeder Schritt führt Aktionen aus, läuft eine feste Zeit (`wait_s`) oder bis eine Bedingung gilt
(`until` mit `timeout_s`) und prüft danach `expect`. Alle Aktionen und Bedingungen stehen in der
[Referenz](/referenz/szenarien).

## Ausführen

```sh
railit scenario run tests/scenarios/*.toml              # lokal
railit scenario run --channels 3 tests/scenarios/*.toml # mit 2-von-3
railit scenario run --node tests/scenarios/*.toml       # auf Knoten des sicheren Rechners
railit scenario run --journal tests/scenarios/*.toml    # mitschreiben und abspielen
```

Im Repository liegen über zwanzig Szenarien: Einfahrt, Durchfahrt, feindliche Fahrstraßen,
Signalhaltfall, Einfahrt in eine zurückgenommene Fahrstraße, Weiche läuft nicht um, gestörte
Gleisfreimeldung, Durchrutschweg und Flankenschutz, Bahnübergang, Ersatzsignal, unerlaubte
Besetzung, verfallende Hilfshandlung, Streckenblock der Nordbahn, Fahrplan, Disposition, ETCS
mit Notbremsung, EULYNX/RaSTA und Redundanz. Die CI lässt jedes in allen Betriebsarten laufen.
