# Netzformat (rail-it-TOML)

rail-it-TOML ist das kompakte, von Hand schreibbare Netzformat. Unbekannte Schlüssel weist der
Leser ab; ein ungültiges Netz meldet er mit **allen** Fehlern auf einmal. Einführung:
[Netze beschreiben](../leitfaden/netze); Beispiele: `examples/beispielstadt.toml` und
`examples/nordbahn.toml`.

## Aufbau

| Tabelle | Pflicht | Inhalt |
|---------|---------|--------|
| `[network]` | ja | Kopf |
| `[[terminal]]` | | Netzenden |
| `[[track]]` | | Gleise |
| `[[point]]` | | Weichen |
| `[[link]]` | | Verbindungen zwischen Ports |
| `[[signal]]` | | Signale und ETCS-Haltetafeln |
| `[[level_crossing]]` | | Bahnübergänge |
| `[[crossing]]` | | Kreuzungen ohne bewegliche Teile |
| `[[tvd]]` | | Gleisfreimeldeabschnitte |
| `[[area]]` | | Stellbereiche (je einer pro Sicherheitskern) |
| `[[station]]` | | Betriebsstellen mit Bahnsteiggleisen für die Betriebsführung |

Namen sind frei wählbar, müssen aber je Elementart eindeutig sein. Längen und Positionen sind
Meter als Dezimalzahl und werden auf Millimeter gerundet; negative oder nicht endliche Werte
sind ungültig.

## `[network]`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name des Netzes; steht im Plan und im Journal |

## `[[terminal]]` – Netzende

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `kind` | `boundary` · `buffer_stop` · `border` | Netzgrenze (Züge fahren ein und aus), Prellbock, Grenze zu einem Nachbarbereich |
| `initial` | bool, Vorgabe `false` | Nur bei `border`: Dieser Bereich beginnt mit der Erlaubnis für die Strecke |

Port: `end`.

## `[[track]]` – Gleis

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `length_m` | Zahl | Länge in Metern |

Ports: `a`, `b`. Die Richtung `a_to_b` läuft von `a` nach `b`; Positionen auf dem Gleis zählen
ab `a`.

## `[[point]]` – Weiche

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `diverging` | `left` · `right` | Abzweigender Strang (optional) |
| `diverging_speed_kmh` | Zahl, Vorgabe 40 | Geschwindigkeit im abzweigenden Strang; nur mit `diverging` |

Ports: `tip` (Spitze), `left`, `right`.

## `[[link]]` – Verbindung

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `from`, `to` | `"<Element>.<Port>"` | Die beiden verbundenen Ports |

Jeder Port wird genau einmal verbunden. Elementnamen dürfen selbst Punkte enthalten; der Port
ist der Teil nach dem letzten Punkt.

## `[[signal]]`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `track` | Text | Gleis, auf dem das Signal steht |
| `position_m` | Zahl | Position ab Port `a`, höchstens die Gleislänge |
| `direction` | `a_to_b` · `b_to_a` | Wirkrichtung |
| `kind` | `main` · `distant` · `combined` · `shunting` · `etcs_stop_marker` | Haupt-, Vor-, Mehrabschnitts-, Rangiersignal, ETCS-Haltetafel |
| `system` | `ks` (Vorgabe) · `hv` | Signalsystem |
| `auto` | bool, Vorgabe `false` | Selbstblocksignal: die Zuglenkung stellt selbsttätig |

Hauptsignale stehen an einer Grenze der Gleisfreimeldung (REQ-PLN-010).

## `[[level_crossing]]` – Bahnübergang

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `track` | Text | Gleis |
| `position_m` | Zahl | Position ab Port `a` |

## `[[crossing]]` – Kreuzung

Zwei Gleise schneiden sich niveaugleich, ohne verbunden zu sein (Kreuzung ohne bewegliche
Teile). Beide Kreuzungsstellen müssen auf verschiedenen Gleisen im **selben**
Gleisfreimeldeabschnitt liegen (REQ-TOP-016); Fahrten über die eine und die andere
Kreuzungsstelle schließen sich damit über den Abschnitt aus. Kreuzungsweichen (EKW, DKW) gibt
es noch nicht. Beispiel: `examples/kreuzung.toml`.

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `legs` | zwei `{ track, position_m }` | Kreuzungsstellen: Gleis und Position ab Port `a` |

## `[[tvd]]` – Gleisfreimeldeabschnitt

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `elements` | Liste | Gleise und Weichen des Abschnitts |

Jedes Gleis und jede Weiche gehört zu genau einem Abschnitt.

## `[[area]]` – Stellbereich

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name des Stellbereichs |
| `tvds` | Liste | Gleisfreimeldeabschnitte des Bereichs |

Ohne `[[area]]` bildet das ganze Netz einen Bereich. Mit Stellbereichen gehört jeder Abschnitt
zu genau einem Bereich; Bereiche stoßen an Netzenden der Art `border` aneinander
([Stellbereiche und Streckenblock](../leitfaden/stellbereiche)).

## `[[station]]` – Betriebsstelle

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Name |
| `platforms` | Liste von `{ name, signals }` | Bahnsteiggleise; `signals` sind die Signale am Gleis, an denen Fahrstraßen enden (meist die Ausfahrsignale beider Richtungen) |

```toml
[[station]]
name = "Adorf"
platforms = [{ name = "1", signals = ["N1", "P1"] }, { name = "2", signals = ["N2", "P2"] }]
```

## Prüfungen

`railit net validate` meldet unter anderem:

- doppelte Namen, unbekannte Elemente und Ports, Ports mehrfach oder gar nicht verbunden,
- nicht zusammenhängende Netze (nur als Warnung),
- Signale und Bahnübergänge außerhalb des Gleises, Hauptsignale innerhalb eines Abschnitts,
- Lücken oder Überschneidungen in der Gleisfreimeldung,
- `initial` außerhalb von `border`, `diverging_speed_kmh` ohne `diverging`,
- Stellbereiche ohne Abschnitte, Abschnitte in keinem oder mehreren Bereichen,
- Bahnsteige mit Signalen, an denen keine Fahrstraße endet.

## PlanPro

`railit net export netz.toml --out netz.ppxml` (oder `--planpro`) schreibt das Netz als
PlanPro 1.10 (`LST_Zustand` mit TOP_Knoten, TOP_Kante, W_Kr_Gsp_Element/-Komponente, Signal,
BUE_Anlage, FMA_Komponente, FMA_Anlage, Gleis_Abschnitt; ohne Geometrie). Alle Befehle lesen
PlanPro 1.9 und 1.10 (`.ppxml`, `.planpro`), auch Planungen mit `LST_Planung` (gelesen wird
der Zielzustand). Abbildung:

| PlanPro | rail-it |
|---------|---------|
| `TOP_Kante` (`TOP_Laenge`, `Kantenname`) | Gleis; Achszählpunkte teilen es (`Name/1`, `Name/2`) |
| `TOP_Knoten` mit `Spitze`/`Links`/`Rechts` | Weiche; Name aus `W_Kr_Gsp_Element`, abzweigender Strang gegenüber `Stammgleis`, Geschwindigkeit aus `Geschwindigkeit_L`/`_R` |
| Knoten mit einer Kante: `Ende` / sonst | Prellbock / Netzgrenze |
| `Signal` mit `Signal_Real_Aktiv_Schirm` | `Hauptsignal`, `Hauptsperrsignal`, `Zugdeckungssignal` → Hauptsignal; `Mehrabschnitts(sperr)signal` → Mehrabschnittssignal; `Vorsignal(wiederholer)` → Vorsignal; `Sperrsignal` → Rangiersignal; `Signalsystem` Ks oder HV; `Signal_Funktion` `Block_Signal` → Selbstblock |
| `BUE_Anlage` | Bahnübergang |
| `W_Kr_Gsp_Komponente` mit `Kreuzung` und zwei Lagen | Kreuzung (Kreuzungsweichen: Fehler) |
| `FMA_Komponente` (`ID_FMAgrenze`), `FMA_Anlage` → `Gleis_Abschnitt` | Freimeldegrenze, Name des Abschnitts |

Hauptsignale müssen wie in jedem rail-it-Netz an einer Freimeldegrenze stehen (REQ-PLN-010);
liegt die Grenze in der Planung einige Meter hinter dem Signal, meldet die Projektierung das.
Fiktive Signale, Gleissperren und Signale ohne Lichtsignalschirm werden mit Warnung
übergangen; Kreuzungen und andere Signalsysteme sind Fehler. Stellbereiche und Betriebsstellen
kennt der Export nicht. Hin und zurück entsteht ein bitgleicher Plan (REQ-TOP-015).
Beispiel: `examples/abzweig.ppxml`.

## railML 3

`railit net export netz.toml --out netz.railml` schreibt dasselbe Netz als railML 3; alle
Befehle lesen `.railml` ebenso wie `.toml`. Was railML nicht eindeutig angibt, meldet der
Import, statt es zu raten. Hin und zurück entsteht ein bitgleicher Plan (REQ-TOP-014).
