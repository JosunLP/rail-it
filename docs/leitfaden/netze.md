# Netze beschreiben

Ein Netz besteht aus **Gleisen**, **Weichen** und **Netzenden**, die über Ports verbunden sind,
dazu **Signalen**, **Bahnübergängen** und **Gleisfreimeldeabschnitten**. rail-it liest es aus
einem kompakten TOML-Format oder aus railML 3. Die vollständige Beschreibung aller Schlüssel
steht in der [Referenz](/referenz/netzformat).

## Topologie mit Ports

Jedes Element hat feste Ports:

| Element | Ports |
|---------|-------|
| Gleis (`track`) | `a`, `b` – Richtung `a_to_b` läuft von `a` nach `b` |
| Weiche (`point`) | `tip` (Spitze), `left`, `right` |
| Netzende (`terminal`) | `end` |

Verbindungen (`link`) verbinden genau zwei Ports. Die Beispielstadt:

```toml
[network]
name = "Beispielstadt"

[[terminal]]
name = "GW"
kind = "boundary"          # Netzgrenze; außerdem buffer_stop, border

[[track]]
name = "west"
length_m = 1000

[[point]]
name = "W1"
diverging = "right"        # abzweigender Strang
diverging_speed_kmh = 40

[[link]]
from = "GW.end"
to = "west.a"

[[link]]
from = "west.b"
to = "W1.tip"
```

## Signale und Bahnübergänge

Signale stehen auf einem Gleis, mit Position ab Port `a` und einer Wirkrichtung:

```toml
[[signal]]
name = "A"
track = "west"
position_m = 1000
direction = "a_to_b"
kind = "combined"          # main, distant, combined, shunting, etcs_stop_marker
system = "ks"              # ks oder hv

[[level_crossing]]
name = "BÜ1"
track = "east"
position_m = 500
```

::: tip Hauptsignale an Abschnittsgrenzen
Ein Hauptsignal muss an einer Gleisfreimeldegrenze stehen – in der Praxis am Gleisende. Die
Validierung meldet Signale an anderen Stellen (REQ-PLN-010).
:::

## Gleisfreimeldung

Jedes Gleis und jede Weiche gehört zu genau einem Gleisfreimeldeabschnitt; Lücken weist die
Validierung ab:

```toml
[[tvd]]
name = "TW1"
elements = ["W1"]
```

## Prüfen und ansehen

```sh
railit net validate netz.toml        # alle Fehler auf einmal
railit net info netz.toml            # Elemente, Signale, Abschnitte
railit net layout netz.toml --out netz.svg
railit net export netz.toml --out netz.railml   # als railML 3
```

`validate` prüft unter anderem: eindeutige Namen, jede Verbindung genau einmal, keine offenen
Ports, zusammenhängendes Netz, Signale auf existierenden Gleisen innerhalb der Gleislänge,
lückenlose Gleisfreimeldung, Weichen mit genau einem abzweigenden Strang.

## railML 3

```sh
railit net validate netz.railml
railit plan compile netz.railml --out plan.rplan --key railit.key
```

Der Import ergänzt und rät nichts: Was railML nicht eindeutig angibt, wird gemeldet. Ein
exportiertes Netz liest rail-it unverändert wieder ein; der Plan ist bitgleich (REQ-TOP-014).

## Große Netze erzeugen

Für Lasttests erzeugt `railit net generate` einen Korridor aus vielen Bahnhöfen:

```sh
railit net generate --stations 236 --out korridor.toml   # ≈ 5000 Elemente
```
