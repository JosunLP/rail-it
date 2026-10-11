# Anforderungen

Anforderungen tragen eine stabile ID der Form `REQ-<BEREICH>-<NR>`. Code, Tests und
Fehlermeldungen verweisen auf diese IDs; daraus entsteht die Rückverfolgbarkeit
Anforderung → Code → Test (Konzept, Abschnitt 16).

| Datei | Bereich |
|-------|---------|
| [topologie.md](topologie.md) | `REQ-TOP`: Netzmodell und seine Validierung |

**Regeln**

- IDs werden nie wiederverwendet. Entfällt eine Anforderung, wird sie als *entfallen*
  markiert.
- Jede Anforderung nennt, welche Gefährdung sie beherrscht (`H-…`, siehe
  [Gefährdungsliste](../safety/hazard-log.md)), und wie sie nachgewiesen wird.
