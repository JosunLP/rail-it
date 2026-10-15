# Fehlermöglichkeits- und Einflussanalyse (FMEA)

Diese FMEA betrachtet die Funktionsblöcke von rail-it 1.0 und fragt je Block: Wie kann er
versagen, was folgt daraus, woran wird es erkannt, was beherrscht es? Sie ergänzt die
[Gefährdungsliste](hazard-log.md), die von den Gefährdungen am Gleis ausgeht, um den Blick von
unten, von den Bauteilen der Software.

::: warning Geltungsbereich
Die Analyse ist **qualitativ**. Sie bewertet Auftreten und Bedeutung in drei Stufen und weist
jede Maßnahme mit Anforderung und Nachweis nach. Eine quantitative Bewertung (Ausfallraten,
THR nach EN 50126/50129) setzt die Zielhardware und Betriebsdaten voraus und ist eine
[offene Voraussetzung jeder Zulassung](safety-case.md#teil-7--offene-punkte-bis-zu-einer-zulassung).
Die Analyse ist nicht unabhängig begutachtet.
:::

## Bewertung

| Stufe | Bedeutung (B) | Auftreten nach Maßnahme (A) |
|-------|---------------|-----------------------------|
| 3 | kann zu einem Unfall führen (Zusammenstoß, Entgleisung, Gefährdung von Personen) | denkbar, Maßnahme greift nur organisatorisch |
| 2 | Betriebsbehinderung, Sicherheit unberührt (sicherer Zustand, Zug hält) | technisch beherrscht, Restfehler nur bei Mehrfachfehlern |
| 1 | nur Anzeige oder Komfort | praktisch ausgeschlossen (formal nachgewiesen oder durch Struktur unmöglich) |

**Restrisiko** ist B × A nach Maßnahme. Werte ab 6 sind hervorgehoben und in Teil 7 des
Sicherheitsnachweises als offen geführt.

## Kern der Fahrwegsicherung (`railit-kernel`, SIL 4)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Feindliche Fahrstraßen gleichzeitig eingestellt | Zusammenstoß | Invariantenprüfung nach jedem Zyklus | Feindlichkeitsmatrix aus dem Plan, Prüfung vor dem Einstellen (REQ-IXL-002/003), unabhängige Invariantenprüfung nach jedem Zyklus, sicherer Zustand bei Verletzung (REQ-IXL-008, REQ-IXL-023); Kani-Nachweise (REQ-SAF-006) | 3 | 1 | 3 |
| Fahrt ohne verschlossene Weiche in Endlage | Entgleisung | Überwachung der Endlage je Zyklus | Fahrt nur, solange alle Weichen in Fahr-, Durchrutsch- und Flankenschutzweg überwacht in Lage sind (REQ-IXL-005, REQ-IXL-012/013); Rücknahme bei Verlust | 3 | 1 | 3 |
| Weiche läuft unter dem Zug um | Entgleisung | Belegung des Abschnitts | Stellbefehl nur für nicht verschlossene Weichen (REQ-IXL-004); gekoppelte Weichen nur gemeinsam (REQ-IXL-026) | 3 | 1 | 3 |
| Fahrstraße löst vorzeitig auf | Flankenfahrt, Gegenfahrt | Teilauflösung nur nach Folge frei → besetzt → frei | Zugbewirkte Auflösung in Fahrtrichtung (REQ-IXL-007), Zeitverschluss bei Rücknahme (REQ-IXL-006) | 3 | 1 | 3 |
| Störung einer Meldung bleibt unbemerkt | Fahrt auf veralteter Grundlage | Alter jeder Meldung (Meldezeit) | Ohne aktuelle Meldung gilt das Element als gestört, gestört wie besetzt (REQ-IXL-009) | 3 | 1 | 3 |
| Panik, Überlauf, Endlosschleife im Kern | Ausfall des Kerns | `forbid(unsafe)`, Lints, Fuzzing; Watchdog | Keine Allokation, keine Panikpfade (REQ-IXL-001; Lints, Fuzzing), Watchdog (REQ-SAF-005) | 2 | 1 | 2 |
| Nichtdeterministisches Verhalten | Kanäle weichen ab | Voter | Deterministischer Kern ohne Uhr und Zufall (REQ-IXL-001); Abweichung → sicherer Zustand (REQ-SAF-001) | 2 | 1 | 2 |

## Plan und Projektierung (`railit-planning`, `railit-plan`)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Generator erzeugt falsche Fahrstraße (fehlender Flankenschutz, falsche Feindlichkeit) | Zusammenstoß, Flankenfahrt | Unabhängige Gegenprüfung | Unabhängig aufgebaute Gegenprüfung gegen das Netz (REQ-PLN-013), Vier-Augen-Freigabe mit Gegenprüfung durch den Prüfer (REQ-PLN-022) | 3 | 2 | **6** |
| Netzbeschreibung entspricht nicht der Anlage | Fahrt auf falscher Grundlage | Plan-Netz-Abgleich beim Start; Prüfung gegen Planungsdaten | Validierung beim Import, Start nur mit zusammengehörigem Plan und Netz (REQ-SVC-001); Prüfung gegen reale Planungen offen | 3 | 2 | **6** |
| Plan-Image verfälscht (Speicher, Datei) | beliebig | SHA-256 beim Laden, zyklisch im Betrieb | Prüfsumme (REQ-PLN-003), zyklische Speicherprüfung (REQ-SAF-008) | 3 | 1 | 3 |
| Fremder Plan untergeschoben | beliebig | Signatur, Freigaben | Ed25519-Signatur (REQ-PLN-004), Freigaben verschiedener Prüfer (REQ-PLN-022) | 3 | 1 | 3 |
| Kopplung einer Kreuzungsweiche falsch | Entgleisung | Symmetrieprüfung beim Laden | Kopplungstabelle im Plan v5, Gegenprüfung über die Lagenpaare (REQ-PLN-021) | 3 | 1 | 3 |

## Sicherheitsschicht und sicherer Rechner (`railit-safety`, `railit-embedded`)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Bitfehler im Zustand eines Kanals | falsche Ausgabe | Voter (Ausgaben und Zustands-Hash) | 2-von-2 / 2-von-3 (REQ-SAF-001/002) | 3 | 1 | 3 |
| Bitfehler im Plan oder im Speicherpfad | falsche Ausgabe aller Kanäle | Zyklische Speicherprüfung | Plan-Prüfsumme und Marsch-Test je Zyklus (REQ-SAF-008); ECC und Anlauftest auf Zielhardware offen | 3 | 2 | **6** |
| Gemeinsamer Fehler aller Kanäle (Compiler, Hardware) | falsche Ausgabe | nicht durch Vergleich | Diversität fehlt heute (gleicher Code, gleicher Compiler); qualifizierte Toolchain und getrennte Hardware offen | 3 | 2 | **6** |
| Kanal hängt oder rechnet zu lange | Ausgaben fehlen | Watchdog | Sicherer Zustand (REQ-SAF-005) | 2 | 1 | 2 |
| Voter selbst fehlerhaft | falsche Weitergabe | Kani-Nachweis für alle Eingaben | Formal nachgewiesen | 3 | 1 | 3 |

## Übertragung (`railit-rasta`, `railit-adapters`)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Verfälschte Nachricht | falscher Stellbefehl oder Meldung | Sicherheitscode | MD4-Sicherheitscode (REQ-COM-001); Stärke nach Zulassungsanforderung offen | 3 | 2 | **6** |
| Wiederholung, Vertauschung, Verlust | veraltete Meldung | Sequenznummern, Quittung | RaSTA-Wiederholung, Lieferung lückenlos in Reihenfolge (REQ-COM-003/007) | 3 | 1 | 3 |
| Einfügung, Maskerade | fremde Befehle | Absender- und Empfängerkennung | Abweisung fremder Kennungen (REQ-COM-002) | 3 | 1 | 3 |
| Verzögerung | veraltete Meldung | Zeitstempel, `t_max` | Abbau nach `t_max` (REQ-COM-004/005) | 3 | 1 | 3 |
| Ausfall eines Übertragungsnetzes | Verbindungsabbau | Diagnose je Kanal | Redundanzschicht über bis zu vier Kanäle (REQ-COM-011) | 2 | 1 | 2 |
| Zwei Stellwerke stellen am Feld | widersprüchliche Befehle | Umschalter | Führend nur ein Stellwerk je Bereich, Umschaltzeit (REQ-SVC-013) | 3 | 1 | 3 |
| Grenzmeldung zwischen Stellwerken veraltet | Gegenfahrt | Alter der Meldung | Erlaubnisprotokoll mit laufender Nummer, gestörte Grenze hält an (REQ-IXL-017, REQ-COM-009) | 3 | 1 | 3 |

## ETCS (`railit-etcs`, `railit-movement`)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Fahrerlaubnis reicht über Fahrt hinaus | Zug fährt in nicht gesicherten Weg | Ableitung nur aus Fahrstraßen mit Fahrt | MA allein aus dem Kernzustand (REQ-ETCS-002), Verkürzung im selben Zyklus (REQ-ETCS-003) | 3 | 1 | 3 |
| Veraltete Vorschau eines Nachbar-RBC | MA über zurückgenommene Fahrt | Alter und Verbindung | Vorschau gilt höchstens 1,5 s über stehende Verbindung (REQ-ETCS-004) | 3 | 2 | **6** |
| Funkstrecke fällt aus | Zug ohne neue MA | Verfall nach 2 s | Fahrzeuggerät hält vor dem nächsten Signal | 2 | 1 | 2 |
| RBC-Funktion nicht im sicheren Kern | MA falsch berechnet | Simulation, Szenarien | RBC rechnet im Dienst, nicht in den Kanälen; für den realen Betrieb muss das RBC sicher ausgeführt werden | 3 | 2 | **6** |

## Dienst, Bedienung und Betriebsführung (`railitd`, Bedienplatz, `railit-ops`)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Falsche Anzeige am Bedienplatz | Fehlbedienung | Anzeigeüberwachung, Prüfcode | Anzeigeüberwachung (REQ-UI-002), Bestätigungskette mit Prüfcode (REQ-UI-003); zweikanalige sichere Anzeige offen | 3 | 2 | **6** |
| Unberechtigte Bedienung | beliebige Anforderung | Anmeldung, Rollen, Audit | Anforderungen prüft immer der Kern; Rollen, mTLS mit Sperrlisten, Audit (REQ-SEC-001 … 006) | 2 | 1 | 2 |
| Hilfshandlung ohne zweite Person | Freigabe in unsicherer Lage | Zwei-Personen-Regel | Hilfshandlungen nur mit Bestätigung durch einen zweiten Bediener, Zählwerk (REQ-IXL-014, REQ-SEC-003) | 3 | 2 | **6** |
| Falsche Zugnummer, falscher Fahrplan | Fehlentscheidung der Zuglenkung | keine technische | Zuglenkung stellt nur Anforderungen; Kern entscheidet (REQ-OPS-001 ff.) | 2 | 2 | 4 |
| Betriebsführung nach Übernahme unvollständig | Nummern fehlen | Fahrdienstleiter prüft | Laufender Abgleich alle 0,5 s (REQ-OPS-007) | 1 | 2 | 2 |

## Verfügbarkeit (Bereitschaftsrechner, Übernahme)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Hauptrechner fällt aus | Stillstand | Schweigen des Journalstroms | Übernahme mit mitgelesenem Zustand, gedeckt durch die Zusage (REQ-SVC-012/014) | 2 | 1 | 2 |
| Übernahme mit veraltetem Zustand | Fahrt auf veralteter Grundlage | Zusage, Ausgabesperre | Warm nur bei gültiger Zusage, sonst Neustart; Fahrt fällt auf Halt (REQ-IXL-025) | 3 | 2 | **6** |
| Zwei Rechner stellen gleichzeitig | widersprüchliche Befehle | Umschalter am Feld | Führung nur nach Abbau aller Verbindungen des anderen (REQ-SVC-013) | 3 | 1 | 3 |

## Nachvollziehbarkeit (`railit-journal`)

| Fehlerart | Auswirkung | Erkennung | Maßnahme | B | A | Rest |
|-----------|-----------|-----------|----------|---|---|------|
| Journal verfälscht oder neu geschrieben | Hergang nicht belegbar | Hashkette, Siegel, Zeugen | REQ-JRN-001 … 004, REQ-JRN-010 | 2 | 1 | 2 |
| Journal unvollständig (Abbruch) | Lücke | letzte Zeile unvollständig | Abbruch erkannt, nie überschrieben (REQ-JRN-003) | 1 | 1 | 1 |

## Ergebnis

Kein Fehlerbild mit Bedeutung 3 ist allein durch Organisation beherrscht. Die hervorgehobenen
Restrisiken (≥ 6) betreffen Punkte, die die Software allein nicht schließen kann:

1. **Gemeinsame Fehler** der Kanäle – diversitäre Hardware, qualifizierte Toolchain.
2. **Speicher** – ECC, vollständiger RAM-Test beim Anlauf auf der Zielhardware.
3. **Sicherheitscode** von RaSTA – Prüfung der Stärke nach Zulassungsanforderung.
4. **Projektierungsdaten** – Prüfung gegen reale Planungen, Validierung durch den Betreiber.
5. **RBC** – Ausführung der RBC-Funktion auf dem sicheren Rechner, Prüfung mit realen
   Fahrzeuggeräten.
6. **Bedienplatz** – ergonomische Prüfung, ggf. zweikanalige sichere Anzeige.
7. **Übernahme** – Zeitbedingungen auf der Zielhardware.

Sie stehen in [Teil 7 des Sicherheitsnachweises](safety-case.md#teil-7--offene-punkte-bis-zu-einer-zulassung).
