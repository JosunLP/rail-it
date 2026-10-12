# Anforderungen an Zugang und Nachvollziehbarkeit (`REQ-SEC`)

Umgesetzt in `apps/railitd` (`auth.rs`, `tls.rs`, `service.rs`) und
`crates/railit-journal` (`audit.rs`). Nachweise in `apps/railitd/tests/security.rs`,
`apps/railitd/tests/grpc.rs`, den Modultests und `apps/railit/tests/cli.rs`.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-SEC-001 | Jeder Benutzer meldet sich einzeln an: mit Token (gespeichert wird nur dessen SHA-256) oder mit Client-Zertifikat. Die Benutzerdatei wird beim Start geprüft: Namen, Token, Zertifikatsnamen und Bedienernummern sind eindeutig, jeder Benutzer hat eine Rolle und eine Anmeldeart, Stellbereiche sind bekannt. | H-006 | `users_file_is_validated`, `roles_areas_and_audit_with_tokens`, `token_and_audit_verification` |
| REQ-SEC-002 | Mit TLS ist die Verbindung verschlüsselt; mit Client-CA kommt ohne gültiges Client-Zertifikat keine Verbindung zustande, und der Common Name bestimmt den Benutzer. REST und gRPC nutzen dieselbe Verbindung (ALPN). | H-006 | `mutual_tls_identifies_users_by_certificate` |
| REQ-SEC-003 | Jede Handlung verlangt das Recht ihrer Rolle und – für Bedienungen am Stellwerk – die Zuständigkeit für den Stellbereich. Die Bedienernummer für die Zwei-Personen-Regel kommt aus der Benutzerdatei. | H-006 | `roles_grant_their_permissions`, `roles_areas_and_audit_with_tokens`, `interlocking_over_grpc` |
| REQ-SEC-004 | Das Auditprotokoll hält jede Bedienung mit Benutzer, Zeit, Ziel, Stellbereich und Ergebnis fest, auch abgewiesene Zugriffe, in einer Hashkette. Es wird über Neustarts fortgesetzt; eine manipulierte Datei verhindert den Start; ist es nicht schreibbar, werden Bedienungen abgewiesen. | H-006, H-011 | `chain_detects_tampering`, `reopened_log_continues_the_chain`, `roles_areas_and_audit_with_tokens`, `token_and_audit_verification` |
| REQ-SEC-005 | Ohne Benutzerdatei und ohne Token lauscht der Dienst nur auf der Loopback-Adresse. | H-006 | `open_mode_only_on_loopback` |
