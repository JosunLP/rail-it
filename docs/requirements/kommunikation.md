# Anforderungen an die sichere Übertragung (`REQ-COM`)

Umgesetzt in `crates/railit-rasta` (RaSTA-Sicherheits- und Wiederholschicht, `no_std`, ohne
Allokator) und `crates/railit-adapters` (EULYNX SCI-P, SCI-LS, SCI-TDS, SCI-LX zu simulierten
Objektsteuerungen). Nachweise in `crates/railit-rasta/tests/rasta.rs`, den Szenarien 18 und
19 und in `apps/railitd/tests/api.rs`. Die Bedrohungen folgen EN 50159.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-COM-001 | Verfälschte Nachrichten werden am Sicherheitscode erkannt und verworfen. | H-005, H-014 | `corruption_is_detected`, Szenario 18 |
| REQ-COM-002 | Nachrichten mit fremder Absender- oder Empfängerkennung werden abgewiesen (Einfügung, Maskerade). | H-014 | `foreign_sender_is_rejected` |
| REQ-COM-003 | Der Empfänger erhält Nachrichten genau einmal und in Sendereihenfolge: Wiederholungen werden verworfen, Verlust wird über die Sequenznummer erkannt und durch Wiederholung behoben. Unter beliebigem Verlust, Verdopplung und Vertauschung ist die Zustellung ein lückenloser Anfang der gesendeten Folge – oder die Verbindung wird abgebaut. | H-014 | `replayed_messages_are_discarded`, `lost_messages_are_retransmitted_in_order`, Property-Test `delivery_is_an_in_order_prefix` (CI: 5000 Fälle) |
| REQ-COM-004 | Nachrichten, die älter als `t_max` sind, bauen die Verbindung ab (Verzögerung). | H-014 | `delayed_messages_break_the_connection` |
| REQ-COM-005 | Lebenszeichen halten die Verbindung; ohne Meldung der Gegenstelle wird sie nach `t_max` abgebaut. | H-005, H-014 | `heartbeats_keep_the_connection_alive`, `silence_times_out` |
| REQ-COM-006 | Das Stellwerk gibt dem Kern nur Meldungen von Objektsteuerungen weiter, deren Verbindung steht. Fällt eine Verbindung aus, fehlen ihre Meldungen; der Kern behandelt die Elemente nach der Meldezeit als gestört (REQ-IXL-009) und nimmt die Fahrt zurück. Die Verbindung wird selbsttätig neu aufgebaut, danach melden die Objektsteuerungen den Zustand ihrer Elemente vollständig. | H-005, H-014 | `field_over_eulynx_and_rasta`, Szenarien 18 und 19 |
