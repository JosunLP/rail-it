# Journal und Auditprotokoll

Beide Dateien sind **JSON Lines** mit SHA-256-Hashkette. Wird eine Zeile verändert, entfernt
oder eingefügt, bricht die Kette. Einführung: [Journal und Rückschau](../leitfaden/journal).

## Betriebsjournal

Jede Zeile hat die Form

```json
{"entry": { "type": "…", … }, "prev": "<64 Hex>", "hash": "<64 Hex>"}
```

mit `hash = SHA-256(prev ‖ JSON(entry))`; `prev` der ersten Zeile sind 32 Nullbytes. Gehasht
wird genau der gespeicherte Text von `entry`.

Weil der Kern deterministisch ist, speichert das Journal nur die **Eingaben** jedes Zyklus.
Daraus lässt sich jeder Zustand exakt rekonstruieren.

| `type` | Inhalt |
|--------|--------|
| `header` | Erste Zeile: `format` (`railit-journal`), `version` (4), `network`, `plan_digest`, `cycle_ms`, `rules`, `checkpoint_every`, `snapshot_every` |
| `cycle` | Ein Zyklus: `cycle`, `now_ms`, Eingaben (`inputs` vollständig oder `delta` als Änderung zum vorigen Zyklus), `outputs` (Hash der Ausgaben), an Prüfpunkten `state` (Hash des Kernzustands) |
| `safety` | Meldung der Sicherheitsschicht vor dem Zyklus, in dem sie wirkt: `kind` (`excluded`, `reintegrated`, `tripped`, `reset`, `node`), `detail` |
| `snapshot` | Zustandsabzug nach einem Zyklus: `cycle`, `state` (gesamter Kernzustand) |
| `resync` | Zustandsübernahme (Knotenbetrieb, Übernahme): `cycle`, `now_ms`, `detail`, `state` |
| `seal` | Siegel: `head` (Hash der vorigen Zeile), `key`, `signature` (Ed25519) |

- **Prüfpunkte** kommen alle `checkpoint_every` Zyklen (`--checkpoint-every`); dort stehen
  die Eingaben vollständig.
- **Zustandsabzüge** kommen alle `snapshot_every` Zyklen (zehn Minuten Betrieb). Das Replay
  spielt die Abschnitte dazwischen parallel ab und prüft, dass jeder Abschnitt genau den
  nächsten Abzug ergibt.
- **Siegel** folgen mit `--seal-key` nach dem Kopf und danach regelmäßig. Die Kette zeigt jede
  veränderte Zeile, das Siegel zeigt, dass niemand die Datei als Ganzes neu geschrieben hat.

Gelesen werden auch Journale der Versionen 2 und 3. Zustands-Hashes an Prüfpunkten sind nur
zwischen Programmen gleicher Version und Plattform vergleichbar; die Hashes der Ausgaben sind
plattformunabhängig.

```sh
railit journal verify  betrieb.journal [--key railit.pub --require-seal]
railit journal replay  betrieb.journal --plan plan.rplan
railit journal state   betrieb.journal --plan plan.rplan --at 20
railit journal events  betrieb.journal --plan plan.rplan --from 0 --to 90
railit journal follow  betrieb.journal --plan plan.rplan
```

## Auditprotokoll

Das Auditprotokoll hält **Personen** und ihre Bedienungen fest, auch abgewiesene Zugriffe.
Jede Zeile ist `{"record": …, "prev", "hash"}` mit derselben Kette; mit Schlüssel folgt jedem
Eintrag ein Siegel `{"seal": …, "prev", "hash"}`.

| Feld | Inhalt |
|------|--------|
| `seq` | Laufende Nummer ab 1, über Neustarts fortlaufend |
| `time_ms` | Zeitpunkt in Millisekunden seit 1970 (UTC) |
| `user` | Angemeldeter Benutzer |
| `action` | Handlung, z. B. `set_route` |
| `target` | Ziel (Fahrstraße, Weiche, Zug, …) |
| `area` | Stellbereich |
| `outcome` | `ok`, `rejected: <Grund>`, `denied` oder `error: <Beschreibung>` |

Beim Start prüft `railitd` eine vorhandene Datei und setzt ihre Kette fort. Kann nicht
geschrieben werden, sperrt der Dienst alle weiteren Bedienungen.

```sh
railit audit verify audit.log [--key railit.pub]
railit audit show --last 50
```
