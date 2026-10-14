# Metriken und Leistung

## Metriken

`railitd` liefert unter `/v1/metrics` Metriken im Prometheus-Textformat (Version 0.0.4), mit
derselben Anmeldung wie `/v1/state`:

```yaml
# prometheus.yml
scrape_configs:
  - job_name: railit
    metrics_path: /v1/metrics
    authorization: { credentials: <Token> }
    static_configs:
      - targets: ['stellwerk:7070']
```

Die wichtigsten Gruppen (vollständige Liste in der [API](../api#metriken)):

| Gruppe | Metriken | Achten auf |
|--------|----------|------------|
| Zyklus | `railit_cycles_total`, `railit_cycle_overruns_total`, `railit_cycle_seconds`, `railit_cycle_seconds_max` | Überläufe, p99 der Rechenzeit |
| Bedienung | `railit_requests_total{result,reason}`, `railit_aux_executed_total`, `railit_aux_counter{area}` | Ablehnungen nach Grund, Hilfshandlungen |
| Sicherheit | `railit_alarms_total{alarm}`, `railit_safe_state{area}`, `railit_channels_active{area}`, `railit_channel_exclusions_total`, `railit_safety_trips_total` | Sicherer Zustand, ausgeschlossene Kanäle |
| Fahrstraßen | `railit_routes{area,phase}` | Eingestellte Fahrstraßen je Phase |
| Journal | `railit_journal_ok` | 0 heißt: Journal abgebrochen |
| ETCS | `railit_etcs_emergency_stops_total` | Notbremsungen |
| Knoten | `railit_node_up`, `railit_node_synced{area}`, `railit_node_divergences_total`, … | Nur mit `--node` |
| Ausgabesperre | `railit_replication_gated`, `…_covered`, `…_held`, `…_oldest_seconds`, `…_acks_total`, `…_rejected_acks_total`, `…_lapses_total` | Nur mit `--standby-gate-ms` |

Beispiele für Alarme:

```yaml
- alert: RailitJournalAbgebrochen
  expr: railit_journal_ok == 0
- alert: RailitSichererZustand
  expr: max(railit_safe_state) > 0
- alert: RailitBereitschaftNichtGedeckt
  expr: railit_replication_gated == 1 and railit_replication_covered == 0
  for: 30s
```

`/v1/health` fasst Zustand, Redundanz, Knoten und Bereitschaftsrechner für einfache
Lebendprüfungen zusammen.

## Leistung messen

`railit bench` erzeugt einen Korridor beliebiger Größe, projektiert ihn und misst gegen die
Ziele aus dem [Konzept](../KONZEPT) (Abschnitt 15). Last: alle Elemente melden in jedem
Zyklus, dazu laufend Fahrstraßenanforderungen und Rücknahmen.

```sh
railit bench                                   # 236 Bahnhöfe, 6000 Zyklen
railit bench --stations 236 --cycles 6000 --check
railit bench --stations 20 --check --output json
railit net generate --stations 50 --out korridor.toml
```

Mit `--check` endet der Befehl mit Exit-Code 1, sobald ein Ziel verfehlt ist; die CI nutzt
das. Messung auf 4 Kernen bei 5004 Elementen (236 Bahnhöfe, 1888 Fahrstraßen):

| Kennzahl | Ziel | gemessen |
|----------|------|----------|
| Kernzyklus p99 | < 1 ms | 0,13–0,20 ms |
| Zulassungsprüfung einer Fahrstraße | < 10 µs | ≈ 0,6 µs |
| Replay von 24 h Betrieb (hochgerechnet) | < 60 s | 34–36 s |

Die Invariantenprüfung nach jedem Zyklus ist linear in der Netzgröße. Das Journal speichert
die Eingaben als Änderung zum vorigen Zyklus und enthält alle zehn Minuten einen
Zustandsabzug, sodass das Replay die Abschnitte parallel abspielt
([Journal und Rückschau](./journal)).
