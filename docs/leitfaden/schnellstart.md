# Schnellstart

In zehn Minuten vom Beispielnetz zum bedienten Bahnhof. Vorausgesetzt ist eine
[Installation](./installation); die Beispielnetze liegen dann unter
`~/.local/share/rail-it/examples` (im Quelltext unter `examples/`).

```sh
EX=~/.local/share/rail-it/examples     # Windows: $EX = "$env:LOCALAPPDATA\Programs\rail-it\share\rail-it\examples"
```

## 1. Das Netz prüfen

Die **Beispielstadt** ist ein kleiner Durchgangsbahnhof mit zwei Bahnsteiggleisen, Ks-Signalen
im Westen, H/V-Signalen im Osten und einem Bahnübergang:

```text
   a ▷         A ▶                       N1 ▶     BÜ1        ◀ F       ◁ f
  GW ════ west ════ W1 ╤════ 1 ════╤ W2 ════ east ══╪═══════════ GE
                       ╘════ 2 ════╛
                        ◀ P1/P2   N2 ▶
```

```sh
railit net validate $EX/beispielstadt.toml
railit net info $EX/beispielstadt.toml
railit net layout $EX/beispielstadt.toml --out beispielstadt.svg   # Spurplan als SVG
```

`validate` meldet alle Fehler auf einmal (fehlende Verbindungen, Lücken in der
Gleisfreimeldung, Signale ohne Gleis …); siehe [Netze beschreiben](./netze).

## 2. Projektieren und signieren

```sh
railit plan keygen railit                 # railit.key (geheim) und railit.pub
railit plan compile $EX/beispielstadt.toml --out beispielstadt.rplan --key railit.key
railit plan info beispielstadt.rplan      # Fahrstraßen, Durchrutschwege, Flankenschutz
```

Der Generator leitet alle Fahrstraßen samt Durchrutschwegen, Flankenschutz, Bahnübergängen und
Feindlichkeiten aus der Topologie ab; eine unabhängige Gegenprüfung muss zum selben Ergebnis
kommen. Das Plan-Image ist mit Ed25519 signiert ([Projektierung](./projektierung)).

## 3. Szenarien laufen lassen

```sh
railit scenario run tests/scenarios/*.toml     # im Quelltext; je Szenario OK oder Fehler
```

Szenarien beschreiben in TOML, was passiert und was danach gelten muss – Einfahrt, feindliche
Fahrstraßen, Signalhaltfall, gestörte Gleisfreimeldung, Bahnübergang, Hilfshandlungen
([Szenarien](./szenarien)).

## 4. Den Stellwerksdienst starten

```sh
railitd --network $EX/beispielstadt.toml --plan beispielstadt.rplan \
        --trusted-key railit.pub --speed 10 --journal betrieb.journal
```

`--speed 10` lässt die Simulation zehnmal schneller als Echtzeit laufen; das Journal hält jede
Eingabe fest. Der Dienst lauscht auf <http://127.0.0.1:7878>.

## 5. Im Browser bedienen

<http://127.0.0.1:7878/> öffnen:

- **Fahrstraße einstellen:** Startsignal `A`, dann Zielsignal `N1` anklicken.
- **Zug einsetzen:** in einem zweiten Terminal `railit sim train add RE1 --at A --length 120 --speed 80`.
- **Rücknahme, Weiche umstellen, Hilfshandlungen:** Rechtsklick auf Signal, Weiche oder Abschnitt.
- **Tastatur wie am Stelltisch:** `A N1` (Fahrstraße), `FRT A-N1` (Rücknahme), `W1 L` (Weiche).

![Bedienplatz](../img/bedienplatz.png)

Mehr im Kapitel [Bedienplatz](./bedienplatz).

## 6. Über die Kommandozeile bedienen

```sh
railit route set A-N1
railit state                     # Signalbilder, Verschlüsse, Bahnübergänge
railit watch                     # Ereignisse fortlaufend (Strg+C beendet)

# Hilfshandlung: Bediener 1 fordert an, Bediener 2 bestätigt und nennt sie dabei selbst
railit aux request release-route A-N1 --operator 1
railit aux confirm 0 release-route A-N1 --operator 2
```

## 7. Nachvollziehen

```sh
railit journal verify betrieb.journal
railit journal replay betrieb.journal --plan beispielstadt.rplan
railit journal state  betrieb.journal --plan beispielstadt.rplan --at 20   # Zustand bei t = 20 s
```

Der Bedienplatz zeigt dasselbe in der **Rückschau** ([Journal und Rückschau](./journal)).

## Nächste Schritte

- Ein Netz mit drei Stellbereichen und Fahrplan: [Stellbereiche](./stellbereiche) und
  [Betriebsführung](./betriebsfuehrung) mit der **Nordbahn**.
- Redundanz, sicherer Rechner und Übernahme: [Redundanz](./redundanz),
  [Bereitschaftsrechner und Übernahme](./uebernahme).
