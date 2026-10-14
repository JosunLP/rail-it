# Stellbereiche und Streckenblock

Ein Netz kann in **Stellbereiche** geteilt sein – je einer pro Sicherheitskern. Die Kerne teilen
keinen Zustand; an jeder Grenze tauschen sie in jedem Zyklus eine **Grenzmeldung** aus. Das
Beispiel **Nordbahn** besteht aus zwei Bahnhöfen an einer eingleisigen Strecke:

```text
       Adorf                          Strecke                         Bstadt
 GW ─ Aw ─▶AA ┬─ A1 ─▶AN1 ┬ Ao1 ◀AF─ Ao ═ S1 ─▶Sbk1 ═ S2 ═ Bo ─▶BA─ Bo1 ┬─ B1 ─▶BN1 ┬ Be ◀BF ─ GE
              └─ A2 ─▶AN2 ┘                Sbk2◀                         └─ B2 ─▶BN2 ┘
```

```toml
[[area]]
name = "Adorf"
tvds = ["ATW", "ATW1", "AT1", "AT2", "ATW2", "ATO1", "ATO"]

[[area]]
name = "Strecke"
tvds = ["ST1", "ST2"]
```

## Erlaubniswechsel

An jeder Grenze darf nur die Seite Züge hinüberschicken, die die **Erlaubnis** hält. Die andere
Seite hält ihren Annäherungsabschnitt – von der Grenze bis zum Einfahrsignal samt
Durchrutschweg – verschlossen.

- **Anlauf:** An jeder Grenze ist der Bereich, der in der Netzbeschreibung zuerst steht, für
  den Anlauf bestimmt; er übernimmt die Erlaubnis, nachdem der Nachbar die Einschwingzeit lang
  gemeldet hat, sie nicht zu halten. Die Grenzen selbst entstehen beim Teilen des Netzes
  automatisch dort, wo Abschnitte zweier Bereiche aneinanderstoßen.
- **Abgabe** nur auf Anforderung, ohne Fahrstraße über die Grenze und bei freien, gesicherten
  Annäherungsabschnitten auf beiden Seiten.
- **Übernahme** nur aus einem Angebot mit größerer laufender Nummer.
- **Fahrt über die Grenze** nur mit Erlaubnis und aktueller Meldung des Nachbarn, dass sein
  Annäherungsabschnitt frei und ausschließlich verschlossen ist.
- Auf der **eingleisigen Strecke** gibt der Streckenbereich eine Erlaubnis nur ab, solange kein
  anderer Annäherungsabschnitt den eigenen verschließt – so schicken nie zwei Bahnhöfe Züge in
  dieselbe Strecke.

Property-Tests mit zwei und drei echten Kernen belegen, dass nie beide Seiten einer Grenze die
Erlaubnis halten.

## Nachbarstellwerke über SCI-ILS

Ohne weitere Angabe stellt der Dienst die Grenzmeldungen ideal im Prozess zu. Mit
`--borders ils` laufen sie wie zwischen getrennten Stellwerken als **EULYNX-SCI-ILS**-Telegramme
über **RaSTA** – je Paar benachbarter Bereiche eine Verbindung, deren Strecke sich stören lässt:

```sh
railitd --network nordbahn.toml --plan nordbahn/ --trusted-key railit.pub --borders ils --speed 10 &
railit ils                                        # Verbindungen, Zähler, Störungen
railit ils fault Adorf-Strecke --loss 30 --delay-ms 80   # gestört: RaSTA gleicht aus
railit ils fault Adorf-Strecke --down             # unterbrochen
railit ils fault Adorf-Strecke                    # wieder ungestört
railit scenario run tests/scenarios/22_nordbahn_sci_ils.toml
railit scenario run --ils tests/scenarios/*.toml  # alle Szenarien über SCI-ILS
```

Ist die Verbindung unterbrochen, kommen keine Meldungen des Nachbarn mehr an: Eine Ausfahrt über
die Grenze fällt nach der Meldezeit auf Halt, beide Seiten melden die Grenze gestört, und die
Erlaubnis bleibt, wo sie war – eine Anforderung wartet, bis die Verbindung wieder steht. Ein
Property-Test lässt die drei Kerne der Nordbahn unter zufälligen Störungen (Unterbrechung,
Verlust, Verfälschung, Laufzeiten über `t_max`) Erlaubnisse anfordern und Ausfahrten stellen:
Nie halten beide Seiten einer Grenze die Erlaubnis, nie zeigen Ausfahrten beider Bahnhöfe in die
Strecke Fahrt (REQ-COM-009).

## Bedienen

```sh
railit plan compile nordbahn.toml --out nordbahn/ --key railit.key
railitd --network nordbahn.toml --plan nordbahn/ --trusted-key railit.pub --speed 10

railit areas                              # Bereiche, Grenzen, wer die Erlaubnis hält
railit border request 'Strecke>Adorf'     # Erlaubnis anfordern (> in Anführungszeichen)
railit route set 'AN1->Strecke'           # Fahrstraße über die Grenze
```

Grenzen heißen `Bereich>Nachbar`, Fahrstraßen über eine Grenze `Signal->Nachbar`. Am Bedienplatz
zeigt jede Grenze, wer die Erlaubnis hält; ein Rechtsklick fordert sie an.

![Nordbahn am Bedienplatz](../img/nordbahn.png)
