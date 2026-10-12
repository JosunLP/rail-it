# Anforderungen

Anforderungen tragen eine stabile ID der Form `REQ-<BEREICH>-<NR>`. Code, Tests und
Fehlermeldungen verweisen auf diese IDs; daraus entsteht die Rückverfolgbarkeit
Anforderung → Code → Test (Konzept, Abschnitt 16).

| Datei | Bereich |
|-------|---------|
| [topologie.md](topologie.md) | `REQ-TOP`: Netzmodell und seine Validierung |
| [plan.md](plan.md) | `REQ-PLN`: Projektierung und Plan-Image |
| [kern.md](kern.md) | `REQ-IXL`: Sicherheitskern (Fahrwegsicherung) |
| [fahrerlaubnis.md](fahrerlaubnis.md) | `REQ-MOV`: Signalbegriffe Ks und H/V, ETCS-Fahrerlaubnis |
| [etcs.md](etcs.md) | `REQ-ETCS`: RBC-Funktion für ETCS Level 2 |
| [kommunikation.md](kommunikation.md) | `REQ-COM`: sichere Übertragung (RaSTA, EULYNX SCI) |
| [sicherheitsschicht.md](sicherheitsschicht.md) | `REQ-SAF`: redundante Kanäle, Voter, Watchdog, formale Nachweise, Fuzzing |
| [embedded.md](embedded.md) | `REQ-EMB`: sicherer Rechner (Cortex-R) |
| [journal.md](journal.md) | `REQ-JRN`: Betriebsjournal und Replay |
| [dienst.md](dienst.md) | `REQ-SVC`: Dienst, API und Bedienung |
| [betrieb.md](betrieb.md) | `REQ-OPS`: Betriebsführung (Zugnummern, Fahrplan, Zuglenkung, Disposition) |
| [sicherheit.md](sicherheit.md) | `REQ-SEC`: Benutzer, Rollen, TLS, Auditprotokoll |

**Regeln**

- IDs werden nie wiederverwendet. Entfällt eine Anforderung, wird sie als *entfallen*
  markiert.
- Jede Anforderung nennt, welche Gefährdung sie beherrscht (`H-…`, siehe
  [Gefährdungsliste](../safety/hazard-log.md)), und wie sie nachgewiesen wird.
