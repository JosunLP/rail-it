import { defineConfig } from 'vitepress';

// Dokumentation von rail-it, veröffentlicht unter https://josunlp.github.io/rail-it/.
export default defineConfig({
  lang: 'de-DE',
  title: 'rail-it',
  description: 'Engine für Netz- und Stellwerkmanagement: Sicherheitskern, Stellwerksdienst, Bedienplatz und Werkzeuge.',
  base: '/rail-it/',
  cleanUrls: true,
  lastUpdated: true,
  srcExclude: ['scripts/**'],
  rewrites: {
    'requirements/README.md': 'requirements/index.md',
  },
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/rail-it/logo.svg' }],
    ['meta', { name: 'theme-color', content: '#1f2937' }],
  ],
  // Verweise aus der Doku in den Quelltext (Protokoll, Crates) zeigen auf GitHub.
  ignoreDeadLinks: [/^\.\.\/\.\.\//, /\/proto\//, /^\.\.\/(crates|apps|examples|tests)\//],
  markdown: {
    theme: { light: 'github-light', dark: 'github-dark' },
  },
  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'rail-it',
    nav: [
      { text: 'Leitfaden', link: '/leitfaden/einfuehrung', activeMatch: '/leitfaden/' },
      { text: 'Referenz', link: '/referenz/', activeMatch: '/(referenz|api)' },
      { text: 'Sicherheit', link: '/safety/safety-case', activeMatch: '/(safety|requirements)/' },
      { text: 'Konzept', link: '/KONZEPT' },
      { text: 'Entwicklung', link: '/entwicklung/beitragen', activeMatch: '/entwicklung/' },
      {
        text: 'Releases',
        items: [
          { text: 'Änderungen', link: '/aenderungen' },
          { text: 'GitHub-Releases', link: 'https://github.com/JosunLP/rail-it/releases' },
        ],
      },
    ],
    sidebar: {
      '/leitfaden/': [
        {
          text: 'Erste Schritte',
          items: [
            { text: 'Einführung', link: '/leitfaden/einfuehrung' },
            { text: 'Installation', link: '/leitfaden/installation' },
            { text: 'Schnellstart', link: '/leitfaden/schnellstart' },
            { text: 'Architektur', link: '/leitfaden/architektur' },
          ],
        },
        {
          text: 'Netz und Plan',
          items: [
            { text: 'Netze beschreiben', link: '/leitfaden/netze' },
            { text: 'Projektierung und Signatur', link: '/leitfaden/projektierung' },
            { text: 'Stellbereiche und Streckenblock', link: '/leitfaden/stellbereiche' },
          ],
        },
        {
          text: 'Betrieb',
          items: [
            { text: 'Stellwerksdienst', link: '/leitfaden/dienst' },
            { text: 'Bedienplatz', link: '/leitfaden/bedienplatz' },
            { text: 'Fahrstraßen und Hilfshandlungen', link: '/leitfaden/bedienung' },
            { text: 'Betriebsführung und Fahrplan', link: '/leitfaden/betriebsfuehrung' },
            { text: 'ETCS, EULYNX und railML', link: '/leitfaden/europa' },
            { text: 'Szenarien', link: '/leitfaden/szenarien' },
          ],
        },
        {
          text: 'Sicherheit und Verfügbarkeit',
          items: [
            { text: 'Benutzer, Rollen, TLS', link: '/leitfaden/zugang' },
            { text: 'Redundanz und sicherer Rechner', link: '/leitfaden/redundanz' },
            { text: 'Journal und Rückschau', link: '/leitfaden/journal' },
            { text: 'Bereitschaftsrechner und Übernahme', link: '/leitfaden/uebernahme' },
            { text: 'Metriken und Leistung', link: '/leitfaden/metriken' },
          ],
        },
      ],
      '/referenz/': referenz(),
      '/api': referenz(),
      '/safety/': sicherheit(),
      '/requirements/': sicherheit(),
      '/entwicklung/': [
        {
          text: 'Entwicklung',
          items: [
            { text: 'Mitarbeiten', link: '/entwicklung/beitragen' },
            { text: 'Tests und Nachweise', link: '/entwicklung/pruefung' },
            { text: 'Releases', link: '/entwicklung/releases' },
            { text: 'Dokumentation', link: '/entwicklung/dokumentation' },
          ],
        },
      ],
    },
    socialLinks: [{ icon: 'github', link: 'https://github.com/JosunLP/rail-it' }],
    editLink: {
      pattern: 'https://github.com/JosunLP/rail-it/edit/main/docs/:path',
      text: 'Diese Seite auf GitHub bearbeiten',
    },
    footer: {
      message: 'Lizenz: NCSL v1.0 · Nicht zugelassen für den realen Bahnbetrieb.',
      copyright: '© Jonas Pfalzgraf',
    },
    search: {
      provider: 'local',
      options: {
        translations: {
          button: { buttonText: 'Suchen', buttonAriaLabel: 'Suchen' },
          modal: {
            displayDetails: 'Details anzeigen',
            resetButtonTitle: 'Suche zurücksetzen',
            backButtonTitle: 'Suche schließen',
            noResultsText: 'Keine Treffer für',
            footer: { selectText: 'auswählen', navigateText: 'wechseln', closeText: 'schließen' },
          },
        },
      },
    },
    outline: { label: 'Auf dieser Seite', level: [2, 3] },
    docFooter: { prev: 'Zurück', next: 'Weiter' },
    lastUpdated: { text: 'Zuletzt geändert' },
    darkModeSwitchLabel: 'Darstellung',
    lightModeSwitchTitle: 'Helle Darstellung',
    darkModeSwitchTitle: 'Dunkle Darstellung',
    sidebarMenuLabel: 'Menü',
    returnToTopLabel: 'Nach oben',
    langMenuLabel: 'Sprache',
    notFound: {
      title: 'Seite nicht gefunden',
      quote: 'Dieses Gleis endet am Prellbock.',
      linkLabel: 'Zur Startseite',
      linkText: 'Zur Startseite',
    },
  },
});

function referenz() {
  return [
    {
      text: 'Programme',
      items: [
        { text: 'Überblick', link: '/referenz/' },
        { text: 'railit (CLI)', link: '/referenz/programme/railit' },
        { text: 'railitd (Dienst)', link: '/referenz/programme/railitd' },
        { text: 'railit-field (Feldrechner)', link: '/referenz/programme/railit-field' },
        { text: 'railit-node (Knoten)', link: '/referenz/programme/railit-node' },
      ],
    },
    {
      text: 'Schnittstellen und Formate',
      items: [
        { text: 'REST-API und gRPC', link: '/api' },
        { text: 'Netzformat (rail-it-TOML)', link: '/referenz/netzformat' },
        { text: 'Fahrplan', link: '/referenz/fahrplan' },
        { text: 'Szenarien', link: '/referenz/szenarien' },
        { text: 'Benutzerdatei', link: '/referenz/benutzer' },
        { text: 'Journal', link: '/referenz/journal' },
        { text: 'Umgebungsvariablen', link: '/referenz/umgebung' },
      ],
    },
  ];
}

function sicherheit() {
  return [
    {
      text: 'Sicherheitsnachweis',
      items: [
        { text: 'Safety Case', link: '/safety/safety-case' },
        { text: 'Gefährdungsliste', link: '/safety/hazard-log' },
        { text: 'Rückverfolgbarkeit', link: '/safety/traceability' },
      ],
    },
    {
      text: 'Anforderungen',
      items: [
        { text: 'Übersicht', link: '/requirements/' },
        { text: 'Topologie', link: '/requirements/topologie' },
        { text: 'Plan', link: '/requirements/plan' },
        { text: 'Kern', link: '/requirements/kern' },
        { text: 'Fahrerlaubnis', link: '/requirements/fahrerlaubnis' },
        { text: 'ETCS', link: '/requirements/etcs' },
        { text: 'Kommunikation', link: '/requirements/kommunikation' },
        { text: 'Sicherheitsschicht', link: '/requirements/sicherheitsschicht' },
        { text: 'Embedded', link: '/requirements/embedded' },
        { text: 'Journal', link: '/requirements/journal' },
        { text: 'Dienst', link: '/requirements/dienst' },
        { text: 'Betrieb', link: '/requirements/betrieb' },
        { text: 'Sicherheit', link: '/requirements/sicherheit' },
        { text: 'Bedienplatz', link: '/requirements/bedienplatz' },
      ],
    },
  ];
}
