---
layout: home

hero:
  name: rail-it
  text: Netz- und Stellwerk&shy;management
  tagline: Ein Sicherheitskern für Fahrwegsicherung in Rust – mit Stellwerksdienst, Bedienplatz, ETCS, EULYNX und Übernahme durch einen Bereitschaftsrechner. Für Simulation, Ausbildung und Prüfstände.
  image:
    src: /logo.svg
    alt: rail-it
  actions:
    - theme: brand
      text: Schnellstart
      link: /leitfaden/schnellstart
    - theme: alt
      text: Installation
      link: /leitfaden/installation
    - theme: alt
      text: Auf GitHub
      link: https://github.com/JosunLP/rail-it

features:
  - icon: 🛡️
    title: Sicherheitskern
    details: Fahrwegsicherung mit Durchrutschweg, Flankenschutz, Bahnübergängen und Hilfshandlungen – no_std, ohne unsafe, ohne Panikpfade, mit Invarianten in jedem Zyklus.
    link: /leitfaden/architektur
  - icon: 🗺️
    title: Netze und Pläne
    details: Netze in einem handschreibbaren TOML-Format oder railML 3; Fahrstraßen werden projektiert, unabhängig gegengeprüft und als signiertes Plan-Image geladen.
    link: /leitfaden/netze
  - icon: 🖥️
    title: Dienst und Bedienplatz
    details: railitd mit REST/JSON, gRPC und Ereignisstrom; Web-Bedienplatz mit berechnetem Spurplan, Anzeigeüberwachung und sicherer Bestätigungskette.
    link: /leitfaden/dienst
  - icon: 🚆
    title: Betrieb und Netz
    details: Mehrere Stellbereiche mit Streckenblock, Zugnummernmeldung, Fahrplan, Zuglenkung und Disposition.
    link: /leitfaden/betriebsfuehrung
  - icon: 🇪🇺
    title: Europa
    details: ETCS Level 2 mit RBC-Funktion, Feldanbindung über EULYNX SCI und RaSTA, Import und Export von railML 3.
    link: /leitfaden/europa
  - icon: ♻️
    title: Verfügbarkeit
    details: Redundante Kanäle mit Voter, sicherer Rechner, gesiegeltes Journal mit Rückschau und Übernahme durch einen Bereitschaftsrechner.
    link: /leitfaden/uebernahme
---

::: warning Keine Zulassung
rail-it ist ein **Kandidat für den Laborbetrieb** – Simulation, Ausbildung, Prüfstände. Es ist
nicht für den realen Bahnbetrieb zugelassen. Was bis zu einer Zulassung fehlt, nennt der
[Sicherheitsnachweis](/safety/safety-case#teil-7-–-offene-punkte-bis-zu-einer-zulassung).
:::

## In einer Minute

```sh
curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh
railit net validate ~/.local/share/rail-it/examples/beispielstadt.toml
railit plan compile ~/.local/share/rail-it/examples/beispielstadt.toml --out beispielstadt.rplan
railitd --network ~/.local/share/rail-it/examples/beispielstadt.toml \
        --plan beispielstadt.rplan --allow-unsigned --speed 10
```

Dann <http://127.0.0.1:7878/> öffnen: Start- und Zielsignal anklicken stellt eine Fahrstraße.
Weiter im [Schnellstart](/leitfaden/schnellstart).
