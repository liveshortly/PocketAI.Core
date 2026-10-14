use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use pocket_types::PocketError;
use serde_json::{json, Value};
use tokio::net::TcpListener;
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

fn router(state: AppState) -> Router {
    Router::new()
        .route("/pocket/v1/hello", get(hello))
        .route("/v1/models", get(models))
        .with_state(Arc::new(state))
}

async fn hello(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "host_id": state.host_id,
        "name": "PocketAI",
        "version": env!("CARGO_PKG_VERSION"),
        "cert_fingerprint": "",
    }))
}

async fn models() -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": [],
    }))
}

async fn bind_in_range(addr_for: impl Fn(u16) -> SocketAddr) -> std::io::Result<TcpListener> {
    let mut last_err = None;
    for port in PORT_RANGE {
        match TcpListener::bind(addr_for(port)).await {
            Ok(listener) => return Ok(listener),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no free port in 7480-7499")))
}

pub async fn start(cfg: ServerConfig, host_id: String) -> Result<ServerHandle, PocketError> {
    if cfg.lan_enabled && !cfg.pro {
        return Err(PocketError::NotEntitled {
            feature: "lan_serving".into(),
        });
    }

    let ip: IpAddr = if cfg.lan_enabled {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::LOCALHOST.into()
    };
    let listener = bind_in_range(|port| SocketAddr::new(ip, port))
        .await
        .map_err(|e| PocketError::Network {
            message: e.to_string(),
        })?;
    let port = listener.local_addr().map_err(|e| PocketError::Internal {
        message: e.to_string(),
    })?.port();

    let app = router(AppState { host_id });
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
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

    #[tokio::test]
    async fn starts_and_stops_twice() {
        for _ in 0..2 {
            let handle = start(ServerConfig::default(), "host1".into()).await.unwrap();
            assert!(PORT_RANGE.contains(&handle.port));
            let resp = reqwest::get(format!("http://127.0.0.1:{}/pocket/v1/hello", handle.port))
                .await
                .unwrap();
            assert!(resp.status().is_success());
            handle.stop().await;
        }
    }

    #[tokio::test]
    async fn port_fallback_when_busy() {
        let handle1 = start(ServerConfig::default(), "host1".into()).await.unwrap();
        let handle2 = start(ServerConfig::default(), "host2".into()).await.unwrap();
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
        match start(cfg, "host1".into()).await {
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
        let handle = start(cfg, "host1".into()).await.unwrap();
        assert!(handle.lan_enabled);
        handle.stop().await;
    }

    #[tokio::test]
    async fn listener_transition_without_port_collision() {
        // Loopback-only server, then switch to LAN (wildcard) on a fresh handle —
        // must not collide since the first listener fully released its port.
        let loopback = start(ServerConfig::default(), "host1".into()).await.unwrap();
        let port = loopback.port;
        loopback.stop().await;

        let lan = start(
            ServerConfig {
                lan_enabled: true,
                pro: true,
            },
            "host1".into(),
        )
        .await
        .unwrap();
        // Port may be reused once released; the important property is binding succeeds.
        assert!(PORT_RANGE.contains(&lan.port));
        let _ = port;
        lan.stop().await;
    }
}
