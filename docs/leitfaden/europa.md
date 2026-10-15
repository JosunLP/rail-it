# ETCS, EULYNX und railML

## ETCS Level 2

Die RBC-Funktion (`railit-etcs`) leitet Fahrerlaubnisse (Movement Authorities) aus den
Fahrstraßen mit Fahrt ab: bis zum Ende der letzten anschließend gesicherten Fahrstraße, mit
statischem Geschwindigkeitsprofil, über Bereichsgrenzen nur bis zum Einfahrsignal des Nachbarn.
Wird eine Fahrstraße zurückgenommen, verkürzt sich die Fahrerlaubnis sofort; fehlt sie unter
einem fahrenden Zug, folgt eine Notbremsung.

```sh
railit sim train add ICE1 --at AA --length 200 --speed 120 --etcs   # Zug mit Fahrzeuggerät
railit etcs          # angemeldete Züge, Betriebsart, Ende der Fahrerlaubnis
```

Ks- und H/V-Signale bleiben gleichzeitig gültig: Mischbetrieb ist der Normalfall.

### Über Prozessgrenzen

Rechnen benachbarte Stellbereiche in getrennten Prozessen ([Stellbereiche](./stellbereiche)),
meldet das RBC hinter der Grenze dem Nachbarn über SCI-ILS laufend eine
**Fahrerlaubnis-Vorschau** – wie bei der RBC-RBC-Übergabe: wie weit eine Fahrerlaubnis ab
der Grenze reicht, mit EoA, Gefahrpunkt und den passierbaren Signalen (REQ-ETCS-004). Das RBC
davor hängt sie an seine Fahrerlaubnis an; ist die Vorschau älter als 1,5 s oder die Verbindung
gestört, endet die Fahrerlaubnis wieder an der Grenze. Hat der Zug die Grenze passiert,
übernimmt das RBC des Nachbarn ihn mit seiner ersten Positionsmeldung.

Mit einem Feldrechner läuft die Funkstrecke zu den Fahrzeuggeräten über dessen Steuerung
(`--field-control`): `GET /v1/sim/etcs` liefert die Positionsmeldungen,
`POST /v1/sim/etcs/{zug}` mit `{"rbc": "0A000001", "passable": ["BA", …]}` (oder `null` für die
Notbremsung) die Fahrerlaubnis eines RBC. Das Fahrzeuggerät erhält die Vereinigung aller
gültigen Fahrerlaubnisse; nicht erneuerte verfallen nach 2 s.

## EULYNX und RaSTA

Mit `--field eulynx` laufen alle Stellbefehle und Meldungen als EULYNX-SCI-Telegramme über je
eine RaSTA-Verbindung zu simulierten Objektsteuerungen:

| Objektsteuerung | Schnittstelle |
|-----------------|---------------|
| `OC-Weichen` | SCI-P |
| `OC-Signale` | SCI-LS |
| `OC-Gleisfrei` | SCI-TDS |
| `OC-BUe` | SCI-LX |

RaSTA sichert nach EN 50159 gegen Verfälschung, Wiederholung, Vertauschung, Verlust,
Verzögerung und Einfügung (Sicherheitscode, Kennungen, Datenindex, Zeitstempel,
Lebenszeichen). Fällt eine Verbindung aus, gelten ihre Elemente nach der Meldezeit als gestört;
die Objektsteuerung stellt ihre Signale auf Halt.

```sh
railitd … --field eulynx
curl -X POST http://127.0.0.1:7878/v1/field/links/OC-Signale \
     -H 'Content-Type: application/json' -d '{"loss_percent": 20}'   # Strecke stören
```

Die Strecke zu jeder Objektsteuerung lässt sich stören (Verlust, Verfälschung, Verzögerung,
Unterbrechung); `/v1/field/links` zeigt den Zustand. Als eigener Prozess laufen die
Objektsteuerungen im **Feldrechner** `railit-field` ([Übernahme](./uebernahme)).

## railML 3

```sh
railit net export nordbahn.toml --out nordbahn.railml
railit net validate nordbahn.railml
railit plan compile nordbahn.railml --out nordbahn/ --key railit.key
```

Import und Export decken Topologie, Signale, Gleisfreimeldung, Bahnübergänge, Stellbereiche und
Betriebsstellen ab. Der Import ergänzt nichts und rät nichts; was railML nicht eindeutig angibt,
wird gemeldet.
