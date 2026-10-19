use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use pocket_types::PocketError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

const PORT_RANGE: std::ops::RangeInclusive<u16> = 7480..=7499;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
pub struct ServerConfig {
    pub lan_enabled: bool,
    pub pro: bool,
}

#[derive(Clone)]
struct AppState {
    host_id: String,
    cert_fingerprint: String,
}

pub struct ServerHandle {
    pub port: u16,
    pub lan_enabled: bool,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl ServerHandle {
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, join).await;
        }
    }
}

pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cert_der);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn router(state: AppState, extra: Router) -> Router {
    Router::new()
        .route("/pocket/v1/hello", get(hello))
        .with_state(Arc::new(state))
        .merge(extra)
}

async fn hello(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "host_id": state.host_id,
        "name": "PocketAI",
        "version": env!("CARGO_PKG_VERSION"),
        "cert_fingerprint": state.cert_fingerprint,
    }))
}

async fn models() -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": [],
    }))
}

async fn bind_in_range(
    ip: IpAddr,
    rustls_config: &axum_server::tls_rustls::RustlsConfig,
    app: &Router,
) -> std::io::Result<(u16, axum_server::Handle, tokio::task::JoinHandle<()>)> {
    let mut last_err = None;
    for port in PORT_RANGE {
        let addr = SocketAddr::new(ip, port);
        let axum_server_handle = axum_server::Handle::new();
        let axum_handle_clone = axum_server_handle.clone();
        let app_clone = app.clone();
        let config_clone = rustls_config.clone();

        let join = tokio::spawn(async move {
            let _ = axum_server::bind_rustls(addr, config_clone)
                .handle(axum_handle_clone)
                .serve(app_clone.into_make_service())
                .await;
        });

        match axum_server_handle.listening().await {
            Some(bound_addr) => {
                return Ok((bound_addr.port(), axum_server_handle, join));
            }
            None => {
                axum_server_handle.graceful_shutdown(None);
                let _ = join.await;
                last_err = Some(std::io::Error::other(format!("port {port} unavailable")));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no free port in 7480-7499")))
}

pub async fn start(
    cfg: ServerConfig,
    host_id: String,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
) -> Result<ServerHandle, PocketError> {
    start_with_routes(
        cfg,
        host_id,
        Router::new().route("/v1/models", get(models)),
        cert_der,
        key_der,
    )
    .await
}

/// Same as `start`, but merges `extra` (e.g. `/v1/chat/completions`,
/// `/pocket/v1/pair`) into the router first. `pocket-server` doesn't know
/// about those handlers' implementations - whoever owns them (`pocket-ffi`'s
/// `PocketCore`) builds the `Router` and passes it in, since `Router` values
/// compose by `.merge()` without either side depending on the other's types.
pub async fn start_with_routes(
    cfg: ServerConfig,
    host_id: String,
    extra: Router,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
) -> Result<ServerHandle, PocketError> {
    if cfg.lan_enabled && !cfg.pro {
        return Err(PocketError::NotEntitled {
            feature: "lan_serving".into(),
        });
    }

    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_fingerprint = cert_fingerprint(&cert_der);
    let rustls_config = axum_server::tls_rustls::RustlsConfig::from_der(vec![cert_der], key_der)
        .await
        .map_err(|e| PocketError::Network {
            message: e.to_string(),
        })?;

    let ip: IpAddr = if cfg.lan_enabled {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::LOCALHOST.into()
    };

    let app = router(
        AppState {
            host_id,
            cert_fingerprint,
        },
        extra,
    );

    let (port, axum_server_handle, join) =
        bind_in_range(ip, &rustls_config, &app)
            .await
            .map_err(|e| PocketError::Network {
                message: e.to_string(),
            })?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let shutdown_handle = axum_server_handle.clone();
    tokio::spawn(async move {
        let _ = shutdown_rx.await;
        shutdown_handle.graceful_shutdown(None);
    });

    Ok(ServerHandle {
        port,
        lan_enabled: cfg.lan_enabled,
        shutdown_tx: Some(shutdown_tx),
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> (Vec<u8>, Vec<u8>) {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params =
            rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
                .expect("params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        let cert = params.self_signed(&key_pair).expect("self sign");
        (cert.der().to_vec(), key_pair.serialize_der())
    }

    async fn start_test(cfg: ServerConfig, host_id: String) -> (ServerHandle, String) {
        let (cert, key) = test_identity();
        let fp = cert_fingerprint(&cert);
        let handle = start(cfg, host_id, cert, key).await.unwrap();
        (handle, fp)
    }

    async fn get_response(port: u16, path: &str) -> (u16, Vec<u8>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let _ = rustls::crypto::ring::default_provider().install_default();

        #[derive(Debug)]
        struct NoVerify;
        impl rustls::client::danger::ServerCertVerifier for NoVerify {
            fn verify_server_cert(
                &self,
                _end_entity: &rustls::pki_types::CertificateDer<'_>,
                _intermediates: &[rustls::pki_types::CertificateDer<'_>],
                _server_name: &rustls::pki_types::ServerName<'_>,
                _ocsp_response: &[u8],
                _now: rustls::pki_types::UnixTime,
            ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            fn verify_tls12_signature(
                &self,
                _message: &[u8],
                _cert: &rustls::pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn verify_tls13_signature(
                &self,
                _message: &[u8],
                _cert: &rustls::pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                rustls::crypto::ring::default_provider()
                    .signature_verification_algorithms
                    .supported_schemes()
            }
        }

        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify))
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let domain = rustls::pki_types::ServerName::try_from("127.0.0.1")
            .unwrap()
            .to_owned();
        let mut tls_stream = connector.connect(domain, stream).await.unwrap();
        tls_stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = Vec::new();
        tls_stream.read_to_end(&mut buf).await.unwrap();
        let header_str = String::from_utf8_lossy(&buf);
        let status = if header_str.starts_with("HTTP/1.1 200") {
            200
        } else {
            0
        };
        (status, buf)
    }

    async fn get_ok(port: u16, path: &str) -> bool {
        get_response(port, path).await.0 == 200
    }

    #[tokio::test]
    async fn starts_and_stops_twice() {
        for _ in 0..2 {
            let (handle, _) = start_test(ServerConfig::default(), "host1".into()).await;
            assert!(PORT_RANGE.contains(&handle.port));
            assert!(get_ok(handle.port, "/pocket/v1/hello").await);
            handle.stop().await;
        }
    }

    #[tokio::test]
    async fn port_fallback_when_busy() {
        let (handle1, _) = start_test(ServerConfig::default(), "host1".into()).await;
        let (handle2, _) = start_test(ServerConfig::default(), "host2".into()).await;
        assert_ne!(handle1.port, handle2.port);
        handle1.stop().await;
        handle2.stop().await;
    }

    #[tokio::test]
    async fn free_tier_lan_refused() {
        let cfg = ServerConfig {
            lan_enabled: true,
            pro: false,
        };
        let (cert, key) = test_identity();
        match start(cfg, "host1".into(), cert, key).await {
            Err(PocketError::NotEntitled { .. }) => {}
            Err(other) => panic!("expected NotEntitled, got {other:?}"),
            Ok(_) => panic!("expected NotEntitled, got Ok"),
        }
    }

    #[tokio::test]
    async fn pro_lan_access_allowed() {
        let cfg = ServerConfig {
            lan_enabled: true,
            pro: true,
        };
        let (handle, _) = start_test(cfg, "host1".into()).await;
        assert!(handle.lan_enabled);
        handle.stop().await;
    }

    #[tokio::test]
    async fn start_with_routes_merges_in_extra_handlers() {
        use axum::routing::get;
        let extra = Router::new().route("/pocket/v1/status", get(|| async { "ok" }));
        let (cert, key) = test_identity();
        let handle = start_with_routes(ServerConfig::default(), "host1".into(), extra, cert, key)
            .await
            .unwrap();
        assert!(get_ok(handle.port, "/pocket/v1/status").await);
        // Built-in routes still work alongside the merged-in ones.
        assert!(get_ok(handle.port, "/pocket/v1/hello").await);
        handle.stop().await;
    }

    #[tokio::test]
    async fn listener_transition_without_port_collision() {
        let (loopback, _) = start_test(ServerConfig::default(), "host1".into()).await;
        let port = loopback.port;
        loopback.stop().await;

        let (lan, _) = start_test(
            ServerConfig {
                lan_enabled: true,
                pro: true,
            },
            "host1".into(),
        )
        .await;
        assert!(PORT_RANGE.contains(&lan.port));
        let _ = port;
        lan.stop().await;
    }

    #[tokio::test]
    async fn hello_reports_cert_fingerprint() {
        let (handle, fp) = start_test(ServerConfig::default(), "host1".into()).await;
        let (status, body) = get_response(handle.port, "/pocket/v1/hello").await;
        assert_eq!(status, 200);
        let json_start = body
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(0);
        let json: Value = serde_json::from_slice(&body[json_start..]).unwrap();
        assert_eq!(json["cert_fingerprint"], fp);
        assert_eq!(json["host_id"], "host1");
        handle.stop().await;
    }
}
