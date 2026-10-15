# Benutzer, Rollen, TLS

## Betriebsarten der Anmeldung

| Art | Aufruf | Anmeldung |
|-----|--------|-----------|
| offen | ohne `--token`/`--users` | keine; nur auf der Loopback-Adresse erlaubt |
| gemeinsames Token | `--token …` (oder `RAILIT_TOKEN`) | `Authorization: Bearer <token>`, alle Rechte |
| Benutzerverwaltung | `--users users.toml --audit audit.log` | Token oder Client-Zertifikat je Benutzer, Rollen |

## Benutzerdatei

```toml
[[user]]
name = "fdl-adorf"
roles = ["fahrdienstleiter"]
operator = 101                 # Bedienernummer für Hilfshandlungen (eindeutig)
areas = ["Adorf"]              # optional: nur diese Stellbereiche bedienen
token_sha256 = "<64 Hex>"      # Anmeldung mit Token (railit auth token)
cert_cn = "fdl-adorf"          # oder mit Client-Zertifikat (mTLS)
```

```sh
railit auth token fdl-adorf    # erzeugt ein Token und gibt dessen Hash für die Datei aus
```

Details: [Benutzerdatei](/referenz/benutzer).

## Rollen

| Rolle | Rechte |
|-------|--------|
| `beobachter` | lesen |
| `fahrdienstleiter` | lesen, Stellwerk bedienen, Hilfshandlungen |
| `disponent` | lesen, Betriebsführung |
| `instandhaltung` | lesen, Simulation, Auditprotokoll, Sicherheitsschicht |
| `projektierer` | lesen, Auditprotokoll |
| `bereitschaft` | lesen, Journal quittieren (Bereitschaftsrechner) |
| `administration` | alles |

Fehlende Rechte ergeben `403`. Jede Bedienung – auch eine abgewiesene – steht im
**Auditprotokoll** (JSON Lines mit Hashkette):

```sh
railit audit verify audit.log                  # Kette prüfen (offline)
railit audit show --last 50                    # letzte Einträge (online)
```

Kann das Auditprotokoll nicht geschrieben werden, sperrt der Dienst alle weiteren Bedienungen
(*fail closed*).

## TLS und mTLS

```sh
railitd … --users users.toml --audit audit.log \
        --tls-cert server.pem --tls-key server.key --tls-client-ca ca.pem
railit --server https://stw:7878 --ca ca.pem --cert fdl.pem --key fdl.key state
```

Mit `--tls-client-ca` verlangt der Dienst ein Client-Zertifikat; dessen CN identifiziert den
Benutzer (`cert_cn`).

### Sperrlisten

Ein verlorenes oder kompromittiertes Zertifikat sperrt die CA in ihrer Sperrliste (CRL, PEM).
Der Dienst prüft jede Anmeldung dagegen:

```sh
railitd … --tls-client-ca ca.pem --tls-crl ca.crl.pem
```

Der Dienst prüft die Datei alle zwei Sekunden auf Änderungen und lädt sie ohne Neustart neu;
bestehende Verbindungen bleiben bis zu ihrem Ende bestehen. Eine Sperrliste, deren
`nextUpdate` überschritten ist, gilt nicht: Dann – wie bei einer unlesbaren Datei – weist der
Dienst jede Anmeldung mit Zertifikat ab, bis eine gültige Sperrliste vorliegt. Die CA muss
die Sperrliste also rechtzeitig neu ausstellen.

## Siegel

Mit `--seal-key` siegelt der Dienst Journal und Auditprotokoll mit dem Schlüssel des Stellwerks
(Ed25519 über den Stand der Hashkette). Die Kette zeigt jede veränderte Zeile, das Siegel zeigt,
dass niemand die Datei als Ganzes neu geschrieben hat:

```sh
railit journal verify betrieb.journal --key railit.pub --require-seal
railit audit verify audit.log --key railit.pub
```
