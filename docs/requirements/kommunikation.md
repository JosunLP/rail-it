# Anforderungen an die sichere Übertragung (`REQ-COM`)

Umgesetzt in `crates/railit-rasta` (RaSTA-Sicherheits- und Wiederholschicht, `no_std`, ohne
Allokator) und `crates/railit-adapters` (EULYNX SCI-P, SCI-LS, SCI-TDS, SCI-LX zu simulierten
Objektsteuerungen). Nachweise in `crates/railit-rasta/tests/rasta.rs`, den Szenarien 18 und
19 und in `apps/railitd/tests/api.rs`. Die Bedrohungen folgen EN 50159.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-COM-001 | Verfälschte Nachrichten werden am Sicherheitscode erkannt und verworfen. | H-005, H-014 | `corruption_is_detected`, Szenario 18 |
| REQ-COM-002 | Nachrichten mit fremder Absender- oder Empfängerkennung werden abgewiesen (Einfügung, Maskerade). | H-014 | `foreign_sender_is_rejected` |
| REQ-COM-003 | Der Empfänger erhält Nachrichten genau einmal und in Sendereihenfolge: Wiederholungen werden verworfen, Verlust wird über den Datenindex erkannt – auch der Verlust der letzten Nachricht, über den Index im nächsten Lebenszeichen – und durch Wiederholung behoben. Unter beliebigem Verlust, Verdopplung und Vertauschung ist die Zustellung in beiden Richtungen ein lückenloser Anfang der gesendeten Folge – oder die Verbindung wird abgebaut. | H-014 | `replayed_messages_are_discarded`, `lost_messages_are_retransmitted_in_order`, `lost_last_message_is_noticed_by_heartbeat`, Property-Tests `delivery_is_an_in_order_prefix` und `both_directions_deliver_in_order_prefixes` (CI: 5000 Fälle) |
| REQ-COM-004 | Nachrichten, die älter als `t_max` sind, bauen die Verbindung ab (Verzögerung). | H-014 | `delayed_messages_break_the_connection` |
| REQ-COM-005 | Lebenszeichen halten die Verbindung; ohne Meldung der Gegenstelle wird sie nach `t_max` abgebaut. | H-005, H-014 | `heartbeats_keep_the_connection_alive`, `silence_times_out` |
| REQ-COM-006 | Das Stellwerk gibt dem Kern nur Meldungen von Objektsteuerungen weiter, deren Verbindung steht. Fällt eine Verbindung aus, fehlen ihre Meldungen; der Kern behandelt die Elemente nach der Meldezeit als gestört (REQ-IXL-009) und nimmt die Fahrt zurück. Die Verbindung wird selbsttätig neu aufgebaut, danach melden die Objektsteuerungen den Zustand ihrer Elemente vollständig. | H-005, H-014 | `field_over_eulynx_and_rasta`, Szenarien 18 und 19 |
| REQ-COM-007 | Verlust führt zur Wiederholung, nicht zum Abbau: Eine Wiederholung schickt alle unquittierten Nachrichten mit ihrem ursprünglichen Datenindex und ist beliebig oft möglich, ohne doppelt zu liefern; Wiederholanforderungen kreuzen sich nicht zur Blockade. Jede authentische, rechtzeitige Nachricht hält die Lebendüberwachung und die Zeitstempelbestätigung aktuell. `t_max` und `t_h` folgen den Vorgaben der DIN VDE V 0831-200 (1800 ms, 300 ms). Bei 20 % Verlust in beide Richtungen bleibt eine Verbindung bestehen. | H-005, H-014 | `twenty_percent_loss_does_not_break_the_connection`, `both_directions_deliver_in_order_prefixes`, Szenario 18 |
