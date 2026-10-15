# Bereitschaftsrechner und Übernahme

Ein zweiter `railitd` kann den Hauptrechner begleiten und bei dessen Ausfall den Betrieb
übernehmen. Die Seite beschreibt den Aufbau in drei Stufen: Mitlesen, Ausgabesperre und
Übernahme.

## Mitlesen

Der Bereitschaftsrechner folgt dem Journal des Hauptrechners über
`/v1/journal/stream`, prüft jede Zeile (Hashkette, Plan, Kern) und rechnet sie nach:

```sh
railitd --network netz.toml --plan netz.rplan --trusted-key railit.pub \
        --listen 127.0.0.1:7071 --standby http://hauptrechner:7070 --standby-token …
```

Er zeigt den mitgelesenen Zustand über dieselbe API und denselben Bedienplatz an (gelb
gekennzeichnet), weist Bedienungen aber mit `503` ab. Der Hauptrechner braucht dafür
`--journal`; der Zugang des Bereitschaftsrechners hat die Rolle `bereitschaft`
([Benutzer, Rollen, TLS](./zugang)).

`/v1/health` zeigt `role` (`primary` bzw. `standby`) und unter `standby` je Bereich, ob die
Verbindung steht, wie viele Zyklen nachgerechnet und quittiert sind, wie oft neu verbunden
wurde und ob es ein Problem gibt.

## Feldrechner mit Umschalter

Damit ein anderer Rechner das Feld stellen kann, laufen die Objektsteuerungen in einem
eigenen Prozess, dem **Feldrechner** `railit-field`. Haupt- und Bereitschaftsrechner sprechen
ihn über EULYNX SCI und RaSTA auf UDP an:

```sh
railit-field --network netz.toml --listen 0.0.0.0:7900 --control 127.0.0.1:7901
```

Sein **Umschalter** hört immer nur ein Stellwerk (RaSTA-Kennung `0A000001` Hauptrechner,
`0A000002` Bereitschaftsrechner). Ein anderes Stellwerk führt erst, wenn alle Verbindungen
des bisherigen abgebaut sind und die Umschaltzeit (`--switch-ms`, Vorgabe 500 ms) verstrichen
ist. Ein zurückkehrender Hauptrechner bleibt danach ausgesperrt. `GET /v1/status` an der
Steuerung zeigt das führende Stellwerk, die Verbindungen und die Zahl der Wechsel.

## Ausgabesperre

Mit `--standby-gate-ms` gibt der Hauptrechner Stellbefehle, Grenzmeldungen und
ETCS-Fahrerlaubnisse erst frei, wenn der Bereitschaftsrechner den Zyklus quittiert hat
(`POST /v1/journal/ack` mit Zyklus und Hash). Mit jeder Quittung sagt er zu, die Sperre eine
Zeit lang zu halten (`hold_ms`). So kennt der Bereitschaftsrechner alles, was das Feld
erreicht hat.

Bleiben Quittungen länger als die Frist aus, arbeitet der Hauptrechner ohne Sperre weiter
und nimmt sie wieder auf, sobald der Bereitschaftsrechner aufgeholt hat. Den Stand zeigen
`replication` in `/v1/health` und die Metriken `railit_replication_*`
([Metriken](./metriken)).

## Übernahme

```sh
# Feldrechner
railit-field --network netz.toml --listen 0.0.0.0:7900 --control 127.0.0.1:7901

# Hauptrechner
railitd --network netz.toml --plan netz.rplan --trusted-key railit.pub --listen 0.0.0.0:7070 \
        --field remote --field-addr feld:7900 --field-control http://feld:7901 \
        --journal haupt.journal --standby-gate-ms 8000

# Bereitschaftsrechner
railitd --network netz.toml --plan netz.rplan --trusted-key railit.pub --listen 0.0.0.0:7071 \
        --field remote --field-addr feld:7900 --field-control http://feld:7901 \
        --journal bereitschaft.journal --standby http://haupt:7070 --standby-token … \
        --takeover-after-ms 500
```

Schweigt der Strom des Hauptrechners länger als `--takeover-after-ms`, versucht der
Bereitschaftsrechner, am Feldrechner zu führen. Dann gibt es zwei Fälle:

| Fall | Bedingung | Ergebnis |
|------|-----------|----------|
| **Warme Übernahme** | Führung erhalten, bevor die Zusage des Hauptrechners abläuft | Mitgelesener Zustand wird übernommen: Verschlüsse bleiben, Fahrstraßen mit Fahrt stehen auf Halt |
| **Kalte Übernahme** | Zusage abgelaufen oder nie erhalten | Beginn wie nach einem Neustart: alle Elemente unbekannt, alle Signale Halt |

Die Zusage rechnet der Bereitschaftsrechner mit der eigenen Uhr ab dem Absenden der Quittung
und kürzt sie um zehn Prozent und 200 ms. Danach führt er mit eigenem Journal (beginnend mit
der Übernahme), `role` wird `primary`, und der Bedienplatz meldet die Übernahme und nimmt
Bedienungen an.

### Betriebsführung

Der Bereitschaftsrechner holt alle 0,5 s den Zustand der Betriebsführung des Hauptrechners
(`GET /v1/ops/state`): Zugnummern an den Signalen, Fahrplanlage, Verspätung, Rückhaltungen,
verfügte Gleise und Wartezeiten, Vormeldungen benachbarter Stellwerke und den Schalter der
Zuglenkung. Bei der Übernahme setzt er den zuletzt geholten Stand ein, wenn er zum selben
Fahrplan gehört (REQ-OPS-007). Was sich in der letzten halben Sekunde vor dem Ausfall
geändert hat, fehlt; der Fahrdienstleiter prüft die Zugnummern nach der Übernahme. Die
Zugnummern sind Betriebsführung, nicht Sicherung: Fahrstraßen stellt nur der Kern.

::: warning Anwendungsbedingung 9
Bereitschaftsrechner nur mit Feldrechner und Umschalter (`--field remote`) und genau ein
Bereitschaftsrechner mit eigenem Zugang der Rolle `bereitschaft`. Die Frist
`--standby-gate-ms` muss Erkennungszeit (`--takeover-after-ms`), `t_max` von RaSTA (1,8 s)
und Umschaltzeit des Feldrechners deutlich übersteigen – im Test 8 s gegenüber
0,5 s + 1,8 s + 0,5 s. Nach einer Übernahme sind Fahrstraßen mit Fahrt neu einzustellen.
:::

## Nachweis

Ein Property-Test lässt den Hauptrechner an zufälligen Stellen mit fahrenden Zügen ausfallen
und prüft danach: keine Entgleisung, kein Aufschneiden, kein offener Bahnübergang, kein
Überfahren eines Haltfalls. Die Gegenprobe zeigt, dass ein Bereitschaftsrechner mit Rückstand
ohne Sperre einen Bahnübergang vor einem Zug öffnen würde. Ein Test mit drei Prozessen
(Feldrechner, Haupt- und Bereitschaftsrechner) prüft warme und kalte Übernahme Ende zu Ende.
Das Argument steht im [Safety Case](../safety/safety-case) unter G7, die Gefährdung unter H-017
in der [Gefährdungsliste](../safety/hazard-log).
