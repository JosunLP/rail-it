# Einführung

rail-it ist eine Engine für Netz- und Stellwerkmanagement. Im Mittelpunkt steht ein
**Sicherheitskern** für die Fahrwegsicherung: Er prüft jede Anforderung, stellt Fahrstraßen ein,
verschließt Weichen, Abschnitte, Durchrutschwege und Flankenschutz, schließt Bahnübergänge und
gibt Fahrerlaubnisse nur, wenn alles gesichert ist. Um ihn herum liegen die Werkzeuge, die ein
Stellwerk im Betrieb braucht – von der Projektierung bis zur Übernahme durch einen zweiten
Rechner.

## Was rail-it kann

| Bereich | Umfang |
|---------|--------|
| Fahrwegsicherung | Fahrstraßen mit Teilauflösung und Zeitverschluss, Durchrutschwege, Flankenschutz, Bahnübergänge, Ersatzsignal, Erkennung unerlaubter Besetzungen, zählpflichtige Hilfshandlungen mit Zwei-Personen-Regel |
| Signalisierung | Ks und H/V aus dem Kernzustand, ETCS Level 2 mit RBC-Funktion |
| Netz | Mehrere Stellbereiche mit je eigenem Kern, Streckenblock mit Erlaubniswechsel, eingleisige Strecken |
| Betrieb | Zugnummernmeldung, Fahrplan, Zuglenkung, Disposition |
| Feldanbindung | Simulator, EULYNX SCI-P/-LS/-TDS/-LX über RaSTA, Feldrechner über UDP |
| Dienst | REST/JSON, gRPC, Server-Sent Events, Rollen, TLS/mTLS, Auditprotokoll, Metriken |
| Bedienplatz | Spurplan aus der Topologie berechnet, Anzeigeüberwachung, Prüfcode für Hilfshandlungen, Rückschau |
| Sicherheit | Redundante Kanäle mit Voter, Watchdog, Knoten des sicheren Rechners, Kani-Nachweise, Fuzzing |
| Nachvollziehbarkeit | Journal mit Hashkette und Siegel, deterministisches Replay, Zustand zu jedem Zeitpunkt |
| Verfügbarkeit | Bereitschaftsrechner liest mit und übernimmt bei Ausfall, gedeckt durch eine Ausgabesperre |

## Für wen

- **Ausbildung:** Ein Bahnhof oder ein kleines Netz lässt sich im Browser bedienen, mit Zügen,
  Störungen und Hilfshandlungen, ohne Feldtechnik.
- **Prüfstände:** Die Feldanbindung über EULYNX und RaSTA, der Knoten des sicheren Rechners und
  der Feldrechner sind dafür angelegt, später mit realen Objektsteuerungen zu arbeiten.
- **Forschung und Entwicklung:** Kern, Projektierung und Journal sind deterministisch und
  vollständig getestet; jede Anforderung ist bis zu ihrem Nachweis rückverfolgbar.

## Grenzen

rail-it ist **nicht zugelassen**. Das Regelwerk ist ein Lehrregelwerk mit festen Zeiten; die
Regelwerke eines Betreibers, eine qualifizierte Toolchain, die Zielhardware und die unabhängige
Begutachtung fehlen. Der [Sicherheitsnachweis](/safety/safety-case) nennt jeden offenen Punkt.

## Wie es weitergeht

1. [Installation](./installation) – fertige Programme für Linux, macOS und Windows.
2. [Schnellstart](./schnellstart) – Beispielstadt prüfen, projektieren und im Browser bedienen.
3. [Architektur](./architektur) – wie die Teile zusammenspielen.
