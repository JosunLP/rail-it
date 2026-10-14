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
