# Referenz

Die Referenz beschreibt Programme, Schnittstellen und Dateiformate vollständig. Einführungen
und Abläufe stehen im [Leitfaden](../leitfaden/einfuehrung).

## Programme

Die Seiten zu den Programmen entstehen beim Bauen der Dokumentation aus deren Hilfetexten und
passen so immer zum Quelltext.

| Programm | Aufgabe |
|----------|---------|
| [`railit`](./programme/railit) | Kommandozeile: Netze und Pläne prüfen, Szenarien, Journale, Bedienung von `railitd`, Installation pflegen |
| [`railitd`](./programme/railitd) | Stellwerksdienst: Kerne, Feld und Betriebsführung im festen Zyklus; REST/JSON, gRPC, Bedienplatz |
| [`railit-field`](./programme/railit-field) | Feldrechner: simulierte Objektsteuerungen über EULYNX SCI und RaSTA mit Umschalter |
| [`railit-node`](./programme/railit-node) | Knoten des sicheren Rechners: Kern mit Kanälen, Voter und Watchdog über RaSTA |

Exit-Codes von `railit`: `0` Erfolg, `1` ungültig oder abgelehnt, `2` Bedien-, Lese- oder
Verbindungsfehler. Mit `--output json` liefern alle Befehle JSON.

## Schnittstellen und Formate

| Thema | Inhalt |
|-------|--------|
| [REST-API und gRPC](../api) | Endpunkte, Anmeldung, Antworten des Kerns, Ereignisse, Metriken |
| [Netzformat](./netzformat) | rail-it-TOML: Topologie, Signale, Gleisfreimeldung, Stellbereiche |
| [Fahrplan](./fahrplan) | Züge, Halte und Einsetzen in die Simulation |
| [Szenarien](./szenarien) | Kopf, Aktionen und Bedingungen |
| [Benutzerdatei](./benutzer) | Benutzer, Rollen, Token und Zertifikate |
| [Journal](./journal) | Betriebsjournal und Auditprotokoll: Zeilen, Hashkette, Siegel |
| [Umgebungsvariablen](./umgebung) | Alle Variablen der Programme, Installer und des Builds |
