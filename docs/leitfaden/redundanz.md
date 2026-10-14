# Redundanz und sicherer Rechner

## Kanäle und Voter

Jeder Stellbereich kann in **zwei oder drei Kanälen** rechnen, jeder mit eigenem
Zustandsspeicher. Der Voter vergleicht in jedem Zyklus Ausgaben und Zustands-Hash:

- **2-von-2:** Jede Abweichung schaltet ab – sicherer Zustand allein aus dem Plan, alle Signale
  Halt.
- **2-von-3:** Der abweichende Kanal wird ausgeschlossen, die beiden anderen rechnen weiter;
  ein ausgeschlossener Kanal kann den Zustand übernehmen und wieder mitwählen.

Ein **Watchdog** überwacht Rechenzeit und Zyklusabstand.

```sh
railitd … --channels 3 --watchdog-ms 50
railit safety                     # Redundanz, mitwählende Kanäle, Abschaltung
railit safety fault 2 stall       # Prüfung: Kanal 2 fällt aus und wird ausgeschlossen
railit safety reintegrate 2       # Kanal 2 übernimmt den Zustand und wählt wieder mit
railit safety reset               # nach einer Abschaltung (Instandhaltung)
```

Der Voter ist mit Kani für alle Eingaben nachgewiesen.

## Knoten des sicheren Rechners

Der Kern eines Stellbereichs kann auf einem eigenen **Knoten** rechnen: `railit-node` führt ihn
mit Kanälen, Voter und Watchdog im eigenen Takt aus und spricht über RaSTA auf UDP mit
`railitd`. Der Dienst wendet nur die Befehle des Knotens an und führt einen per Digest
geprüften **Spiegel** für Anzeige, Betriebsführung und RBC; weicht er ab, gleicht der Knoten
seinen Zustand ab.

```sh
railit-node --plan beispielstadt.rplan --trusted-key railit.pub --listen 127.0.0.1:7979 &
railitd --network beispielstadt.toml --plan beispielstadt.rplan \
        --trusted-key railit.pub --node 127.0.0.1:7979
# Netze mit Stellbereichen: --node Adorf=…:7979 --node Strecke=…:7980 …
```

Fällt der Knoten aus, zeigen alle Signale Halt, und der Bedienplatz sperrt die Bedienung, bis
er wieder abgeglichen ist. Das Ziel ist ein Cortex-R (`railit-embedded`, `no_std`, ohne
Allokator); `railit-node` ist dieselbe Runtime als Serverprogramm.

Für Prüfung und Ausbildung lassen sich alle Bereiche im Gleichschritt auf Knoten im Dienst
rechnen: `railitd … --node lockstep` bzw. `railit scenario run --node …`.
