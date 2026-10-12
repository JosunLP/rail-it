//! TLS und gegenseitige Anmeldung mit Zertifikaten (mTLS, REQ-SEC-002).
//!
//! Der Dienst spricht HTTP/1.1 und HTTP/2 (für gRPC) auf demselben Port; mit TLS handelt er
//! das Protokoll per ALPN aus. Ist eine Client-CA angegeben, verlangt er von jedem Client ein
//! von ihr ausgestelltes Zertifikat; dessen Common Name (CN) identifiziert den Benutzer.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{Extension, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use rustls::RootCertStore;
use rustls::server::WebPkiClientVerifier;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
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

/// Serverkonfiguration aus Zertifikatskette, Schlüssel und optionaler Client-CA.
pub(crate) fn server_config(
    cert: &Path,
    key: &Path,
    client_ca: Option<&Path>,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let chain = certs(cert)?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("{}: {e}", key.display()))?;
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let builder = match client_ca {
        Some(ca) => {
            let mut roots = RootCertStore::empty();
            for c in certs(ca)? {
                roots.add(c).map_err(|e| format!("{}: {e}", ca.display()))?;
            }
            let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(|e| e.to_string())?;
            builder.with_client_cert_verifier(verifier)
        }
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
