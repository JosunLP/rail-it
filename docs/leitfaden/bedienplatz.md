# Bedienplatz

`railitd` liefert unter <http://127.0.0.1:7878/> einen Web-Bedienplatz aus. Er ist ein reiner
Client der API: Er liest Spurplan und Zustandsbild und bedient über dieselben Pfade wie jeder
andere Client, mit derselben Anmeldung.

![Bedienplatz](../img/bedienplatz.png)

## Spurplan

Der Spurplan wird aus der Topologie **berechnet**, nicht gezeichnet (`/v1/layout`). Signale
zeigen ihren befohlenen Begriff, Abschnitte ihre Belegung und Verschlüsse, Weichen ihre Lage,
Bahnübergänge ihren Zustand, Grenzen den Halter der Erlaubnis. Zugnummern stehen am Signal.

## Bedienen

| Handlung | Maus | Tastatur |
|----------|------|----------|
| Fahrstraße einstellen | Startsignal, dann Zielsignal anklicken | `A N1` |
| Fahrstraße zurücknehmen | Rechtsklick auf das Startsignal | `FRT A-N1` |
| Weiche umstellen | Rechtsklick auf die Weiche | `W1 L` bzw. `W1 R` |
| Erlaubnis anfordern | Rechtsklick auf die Grenze | |
| Hilfshandlung anfordern | Rechtsklick auf Signal, Weiche oder Abschnitt | |

Jede Bedienung ist eine Anforderung; der Kern nimmt sie an oder lehnt sie mit Grund ab (z. B.
„feindliche Fahrstraße“, „Abschnitt nicht frei“). Die Meldungen stehen in der Liste rechts.

## Hilfshandlungen mit zwei Bedienern

Zählpflichtige Hilfshandlungen (Fahrstraße hilfsweise auflösen, Achszähler grundstellen,
Ersatzsignal, Weiche unter Störung umstellen) verlangen einen **zweiten Bediener**:

1. Bediener 1 fordert an (eigene Bedienernummer oben rechts).
2. Bediener 2 öffnet in seinem eigenen Fenster den Bestätigungsdialog, liest die Hilfshandlung
   aus **seiner** Anzeige und gibt das Element **selbst** ein.
3. Der Bedienplatz vergleicht den Prüfcode seiner Anzeige mit dem des Stellwerks; der Kern führt
   die Hilfshandlung nur aus, wenn die Bestätigung sie unverändert wiederholt. Das Zählwerk
   zählt jede ausgeführte Hilfshandlung.

![Bestätigung](../img/bestaetigung.png)

## Anzeigeüberwachung

Kommt 2 s lang kein aktuelles Zustandsbild, deckt der Bedienplatz den Spurplan ab und sperrt
jede Bedienung („Anzeige gestört“). Dasselbe gilt, wenn ein Knoten des sicheren Rechners nicht
abgeglichen ist.

## Rückschau

Mit der Zeitleiste oben lässt sich jeder vergangene Zeitpunkt aus dem Journal zeigen – deutlich
gekennzeichnet, ohne Bedienung und mit den Befehlen und Ereignissen der Minute davor
([Journal und Rückschau](./journal)).

![Rückschau](../img/rueckschau.png)

## Bereitschaftsrechner

Am Bereitschaftsrechner ist der Bedienplatz gelb gekennzeichnet und gesperrt. Übernimmt er den
Betrieb, wird die Bedienung frei und der Bedienplatz meldet, ob mit dem mitgelesenen Zustand oder
nach einem Neustart ([Übernahme](./uebernahme)).

## Sicherheit der Seite

Der Bedienplatz ist SIL 0. Die Seite ist ohne Anmeldung erreichbar, enthält aber keine Daten;
jede Antwort trägt eine strenge Content-Security-Policy. Das Sicherheitsargument stützt sich
nicht auf die Richtigkeit der Anzeige: Jede Regelbedienung prüft der Kern, jede Hilfshandlung
muss die Bestätigung unverändert wiederholen.
