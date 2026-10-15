# Projektierung und Signatur

Aus dem Netz entsteht offline der **Plan**: alle Fahrstraßen mit Weichenlagen, Abschnitten,
Durchrutschwegen, Flankenschutz, Bahnübergängen, Geschwindigkeiten und Feindlichkeiten. Der Kern
lädt ihn als **Plan-Image** – ein Binärformat, das er ohne Kopieren liest, mit SHA-256 über das
ganze Image und einer Ed25519-Signatur.

## Schlüssel

```sh
railit plan keygen railit     # railit.key (geheim, nur Lesen für den Besitzer) und railit.pub
```

Der geheime Schlüssel signiert Pläne und siegelt Journale; der öffentliche Schlüssel prüft sie.
Bewahren Sie ihn getrennt vom Stellwerksrechner auf.

## Plan erzeugen

```sh
railit plan compile netz.toml --out plan.rplan --key railit.key
railit plan info plan.rplan
railit plan verify plan.rplan --key railit.pub --network netz.toml
```

`compile` läuft zweimal: Der **Generator** leitet die Fahrstraßen aus der Topologie ab, eine
**unabhängige Gegenprüfung** rechnet sie auf anderem Weg nach. Nur wenn beide übereinstimmen,
entsteht ein Image (REQ-PLN-013). Netze mit Stellbereichen ergeben ein Verzeichnis mit einem
Plan je Bereich:

```sh
railit plan compile nordbahn.toml --out nordbahn/ --key railit.key
ls nordbahn/       # Adorf.rplan  Bstadt.rplan  Strecke.rplan
```

## Was der Plan enthält

| Teil | Bedeutung |
|------|-----------|
| Fahrstraße | Start- und Zielsignal, Weichen mit Lage, Abschnitte in Fahrtrichtung |
| Durchrutschweg | Abschnitte und Weichen hinter dem Zielsignal |
| Flankenschutz | Weichen in Schutzlage oder Signale in Halt, die feindliche Fahrten fernhalten |
| Bahnübergänge | müssen vor Fahrt geschlossen melden, öffnen erst nach Räumung |
| Geschwindigkeit | aus abzweigenden Weichensträngen, angezeigt als Zs3 bzw. im Vorsignal |
| Feindlichkeiten | Paare von Fahrstraßen, die nie gleichzeitig eingestellt sein dürfen |
| Grenzen | Annäherungsabschnitte und Einfahrsignale an Bereichsgrenzen |

## Plan laden

`railitd` und `railit-node` laden nur signierte Pläne mit dem angegebenen Schlüssel:

```sh
railitd --network netz.toml --plan plan.rplan --trusted-key railit.pub
```

Für Tests und Ausbildung lässt `--allow-unsigned` unsignierte Pläne zu. Beim Start prüft der
Dienst, dass Plan und Netz zusammengehören (gleiche Elemente in gleicher Reihenfolge).

## Freigabe im Vier-Augen-Prinzip

Die Signatur sagt, wer einen Plan erzeugt hat. Bevor ein Plan in Betrieb geht, prüfen ihn
unabhängige Prüfer mit eigenen Schlüsseln und geben ihn frei (REQ-PLN-022):

```sh
railit plan keygen pruefer-a          # einmal je Prüfer
railit plan approve plan.rplan --key pruefer-a.key --creator-key railit.pub \
    --network netz.toml --name "Prüferin A"
```

`approve` prüft die Signatur des Erstellers und den Plan unabhängig gegen das Netz und hängt
die Freigabe an `plan.rplan.approvals` an. Die Freigabe ist eine Ed25519-Signatur über die
Prüfsumme des Plans; jeder neu erzeugte Plan braucht neue Freigaben. Der Ersteller kann
seinen eigenen Plan nicht freigeben.

Der Dienst verlangt die Freigaben beim Start:

```sh
railitd --network netz.toml --plan plan.rplan --trusted-key railit.pub \
    --approver-key pruefer-a.pub --approver-key pruefer-b.pub --approvals 2
```

Gezählt werden nur verschiedene Prüfer aus `--approver-key`. Eine fremde, verfälschte oder
zu einem anderen Plan gehörende Freigabe in der Datei weist den Plan ab. Bei Stellbereichen
braucht jedes `<Bereich>.rplan` seine eigene Freigabedatei. `railit plan verify` prüft
dieselben Bedingungen mit `--approver-key` und `--approvals`.
