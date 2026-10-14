# Journal und Rückschau

## Betriebsjournal

```sh
railitd … --journal betrieb.journal [--checkpoint-every 600] [--seal-key railit.key]
```

Das Journal hält jede Eingabe jedes Zyklus fest – Meldungen des Felds, Anforderungen,
Grenzmeldungen – als JSON Lines mit SHA-256-Hashkette. Dazu kommen Prüfpunkte (Hash des
Zustands), Zustandsabzüge (alle zehn Minuten Betrieb), Meldungen der Sicherheitsschicht und
Siegel. Ein vorhandenes Journal wird nie überschrieben; bricht das Schreiben ab, läuft der
Betrieb weiter und meldet es. Bei mehreren Stellbereichen ist `--journal` ein Verzeichnis mit
`<Bereich>.journal`.

## Prüfen und abspielen

```sh
railit journal verify betrieb.journal [--key railit.pub --require-seal]
railit journal replay betrieb.journal --plan plan.rplan
```

Das Replay rechnet jeden Zyklus mit denselben Eingaben nach und vergleicht Ausgaben und Zustand
an jedem Prüfpunkt. Die Abschnitte zwischen den Zustandsabzügen laufen parallel: 24 Stunden
Betrieb bei 5000 Elementen spielen in etwa 35 Sekunden ab.

## Zustand zu einem Zeitpunkt

```sh
railit journal state  betrieb.journal --plan plan.rplan --at 20          # t = 20 s
railit journal events betrieb.journal --plan plan.rplan --from 0 --to 90 # Befehle, Ereignisse
```

Beides wird ab dem letzten Zustandsabzug davor rekonstruiert, die Ausgaben neu berechnet und
gegen das Journal geprüft. Der Dienst liefert dasselbe unter `/v1/journal/state` und
`/v1/journal/events`; der Bedienplatz zeigt es in der **Rückschau** mit Zeitleiste.

## Mitlesen

```sh
railit journal follow betrieb.journal --plan plan.rplan
```

`follow` liest ein wachsendes Journal mit, prüft jede neue Zeile sofort und rechnet sie nach.
Auf demselben Mitleser beruht der [Bereitschaftsrechner](./uebernahme).

Das Format beschreibt die [Referenz](/referenz/journal).
