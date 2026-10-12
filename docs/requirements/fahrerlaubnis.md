# Anforderungen an die Fahrerlaubnis (`REQ-MOV`)

Umgesetzt in `crates/railit-movement`. Nachweise in `crates/railit-movement/tests/aspects.rs`
und in Szenario 08.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-MOV-001 | Signalbilder richten sich nach dem Signalsystem: Ks (Hp0, Ks1, Ks1 blinkend, Ks2, Zs3, Zs3v) und H/V (Hp0, Hp1, Hp2, Vr0, Vr1, Vr2, Zs3). Eine Geschwindigkeitsbeschränkung der Fahrstraße wird am Startsignal angezeigt (Zs3 bzw. Hp2) und am vorherigen Signal angekündigt (Zs3v bzw. Vr2). | H-002, H-009 | `ks_diverging_entry_shows_zs3_and_is_announced`, `hv_diverging_is_hp2_and_vr2`, `hv_combined_and_distant` |
| REQ-MOV-002 | Ein Hauptsignal zeigt einen Fahrtbegriff genau dann, wenn der Kern Fahrt befohlen hat. Ein Vorsignal oder Vorsignalschirm kündigt Fahrt nur an, wenn das nächste Hauptsignal Fahrt zeigt; an Netzgrenze und Prellbock wird Halt erwartet. | H-009 | Property-Test `pictures_are_consistent`, `signal_drop_turns_announcement_back` |
| REQ-MOV-003 | Das Ersatzsignal erscheint im Signalbild nur, wenn der Kern es eingeschaltet hat (REQ-IXL-015). Die Signalbilder sind Anzeige; sie können keine Fahrt erlauben, die der Kern nicht erlaubt. | H-009 | Architektur: `railit-movement` liest den Kernzustand nur; `display_of_dark_and_substitute` |
