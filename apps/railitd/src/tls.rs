//! TLS und gegenseitige Anmeldung mit Zertifikaten (mTLS, REQ-SEC-002).
//!
//! Der Dienst spricht HTTP/1.1 und HTTP/2 (für gRPC) auf demselben Port; mit TLS handelt er
//! das Protokoll per ALPN aus. Ist eine Client-CA angegeben, verlangt er von jedem Client ein
//! von ihr ausgestelltes Zertifikat; dessen Common Name (CN) identifiziert den Benutzer.
//!
//! Gesperrte Zertifikate stehen in Sperrlisten (CRL, `--tls-crl`, REQ-SEC-006). Der Dienst
//! liest sie bei Änderung neu ein, ohne Neustart; eine abgelaufene oder unlesbare Sperrliste
//! weist jede Anmeldung ab, bis eine gültige vorliegt.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use axum::{Extension, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::server::WebPkiClientVerifier;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer, UnixTime};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::api::PeerIdentity;

/// Höchstdauer des TLS-Handshakes, damit halb offene Verbindungen keine Ressourcen binden.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

fn certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("{}: kein Zertifikat", path.display()));
    }
    Ok(certs)
}

/// Abstand, in dem der Dienst die Sperrlisten auf Änderungen prüft.
const CRL_CHECK: Duration = Duration::from_secs(2);

fn crls(paths: &[PathBuf]) -> Result<Vec<CertificateRevocationListDer<'static>>, String> {
    let mut out = Vec::new();
    for path in paths {
        let before = out.len();
        for crl in CertificateRevocationListDer::pem_file_iter(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
        {
            out.push(crl.map_err(|e| format!("{}: {e}", path.display()))?);
        }
        if out.len() == before {
            return Err(format!("{}: keine Sperrliste", path.display()));
        }
    }
    Ok(out)
}

/// Änderungsmerkmal je Sperrliste: Zeitstempel und Länge.
type Stamp = Option<(SystemTime, u64)>;

fn stamps(paths: &[PathBuf]) -> Vec<Stamp> {
    paths
        .iter()
        .map(|p| {
            std::fs::metadata(p)
                .and_then(|m| Ok((m.modified()?, m.len())))
                .ok()
        })
        .collect()
}

/// Prüfer der Client-Zertifikate gegen CA und (optional) Sperrlisten.
fn webpki_verifier(
    roots: &Arc<RootCertStore>,
    provider: &Arc<rustls::crypto::CryptoProvider>,
    crl_paths: &[PathBuf],
) -> Result<Arc<dyn ClientCertVerifier>, String> {
    let mut builder = WebPkiClientVerifier::builder_with_provider(roots.clone(), provider.clone());
    if !crl_paths.is_empty() {
        // Abgelaufene Sperrlisten gelten nicht: Wer nicht nachliefert, sperrt alle aus.
        builder = builder
            .with_crls(crls(crl_paths)?)
            .enforce_revocation_expiration();
    }
    builder.build().map_err(|e| e.to_string())
}

/// Lädt die Sperrlisten bei Änderung neu (REQ-SEC-006).
#[derive(Debug)]
struct ReloadingVerifier {
    roots: Arc<RootCertStore>,
    provider: Arc<rustls::crypto::CryptoProvider>,
    crl_paths: Vec<PathBuf>,
    hints: Vec<DistinguishedName>,
    schemes: Vec<SignatureScheme>,
    state: RwLock<CrlState>,
}

#[derive(Debug)]
struct CrlState {
    /// `None`: Die Sperrlisten sind unlesbar; jede Anmeldung wird abgewiesen.
    verifier: Option<Arc<dyn ClientCertVerifier>>,
    stamps: Vec<Stamp>,
    checked: Instant,
}

impl ReloadingVerifier {
    fn current(&self) -> Result<Arc<dyn ClientCertVerifier>, rustls::Error> {
        let due = self
            .state
            .read()
            .map_or(true, |s| s.checked.elapsed() >= CRL_CHECK);
        if due && let Ok(mut state) = self.state.write() {
            state.checked = Instant::now();
            let now = stamps(&self.crl_paths);
            if now != state.stamps {
                state.stamps = now;
                match webpki_verifier(&self.roots, &self.provider, &self.crl_paths) {
                    Ok(v) => {
                        tracing::info!("Sperrlisten neu geladen");
                        state.verifier = Some(v);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Sperrliste ungültig: Anmeldungen abgewiesen");
                        state.verifier = None;
                    }
                }
            }
        }
        self.state
            .read()
            .ok()
            .and_then(|s| s.verifier.clone())
            .ok_or_else(|| rustls::Error::General("Sperrliste ungültig".into()))
    }
}

impl ClientCertVerifier for ReloadingVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.hints
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let result = self
            .current()?
            .verify_client_cert(end_entity, intermediates, now);
        if let Err(e) = &result {
            tracing::info!(error = %e, cn = ?common_name(end_entity), "Client-Zertifikat abgewiesen");
        }
        result
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.current()?.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.current()?.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes.clone()
    }
}

/// Prüfer der Client-Zertifikate; mit Sperrlisten neu ladend.
fn client_verifier(
    ca: &Path,
    crl_paths: &[PathBuf],
    provider: &Arc<rustls::crypto::CryptoProvider>,
) -> Result<Arc<dyn ClientCertVerifier>, String> {
    let mut roots = RootCertStore::empty();
    for c in certs(ca)? {
        roots.add(c).map_err(|e| format!("{}: {e}", ca.display()))?;
    }
    let roots = Arc::new(roots);
    let verifier = webpki_verifier(&roots, provider, crl_paths)?;
    if crl_paths.is_empty() {
        return Ok(verifier);
    }
    Ok(Arc::new(ReloadingVerifier {
        hints: verifier.root_hint_subjects().to_vec(),
        schemes: verifier.supported_verify_schemes(),
        roots,
        provider: provider.clone(),
        state: RwLock::new(CrlState {
            verifier: Some(verifier),
            stamps: stamps(crl_paths),
            checked: Instant::now(),
        }),
        crl_paths: crl_paths.to_vec(),
    }))
}

/// Serverkonfiguration aus Zertifikatskette, Schlüssel, optionaler Client-CA und Sperrlisten.
pub(crate) fn server_config(
    cert: &Path,
    key: &Path,
    client_ca: Option<&Path>,
    crl_paths: &[PathBuf],
) -> Result<Arc<rustls::ServerConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let chain = certs(cert)?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("{}: {e}", key.display()))?;
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let builder = match client_ca {
        Some(ca) => builder.with_client_cert_verifier(client_verifier(ca, crl_paths, &provider)?),
        None => builder.with_no_client_auth(),
    };
    let mut config = builder
        .with_single_cert(chain, key)
        .map_err(|e| format!("Zertifikat oder Schlüssel ungültig: {e}"))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Common Name des ersten Zertifikats.
fn common_name(cert: &CertificateDer<'_>) -> Option<String> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref()).ok()?;
    let cn = parsed.subject().iter_common_name().next()?;
    cn.as_str().ok().map(str::to_string)
}

/// Nimmt TLS-Verbindungen an, bis `shutdown` eintritt.
pub(crate) async fn serve(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    app: Router,
    shutdown: impl Future<Output = ()>,
) {
    let acceptor = TlsAcceptor::from(config);
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(error = %e, "Verbindung nicht angenommen");
                    continue;
                }
            },
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let tls = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                Ok(Ok(tls)) => tls,
                Ok(Err(e)) => {
                    tracing::info!(%peer, error = %e, "TLS-Handshake abgewiesen");
                    return;
                }
                Err(_) => {
                    tracing::info!(%peer, "TLS-Handshake: Zeit abgelaufen");
                    return;
                }
            };
            let cn = tls
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|c| c.first())
                .and_then(common_name);
            let service = TowerToHyperService::new(app.layer(Extension(PeerIdentity(cn))));
            if let Err(e) = Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(tls), service)
                .await
            {
                tracing::debug!(%peer, error = %e, "Verbindung beendet");
            }
        });
    }
}
