#[cfg(test)]
mod tests {
    use crate::core::PocketCore;
    use crate::error::PocketError;
    use crate::traits::ChatSink;
    use crate::traits::Engine as PocketKitEngine;
    use crate::traits::ImageEngine;
    use crate::traits::PlatformHooks;
    use crate::traits::SpeechToTextEngine;
    use crate::traits::TextToSpeechEngine;
    use crate::traits::TokenSink as PocketKitTokenSink;
    use crate::types::*;
    use std::sync::{Arc, Mutex};

    struct FakeHooks {
        dir: String,
    }

    impl PlatformHooks for FakeHooks {
        fn keychain_get(&self, _key: String) -> Option<Vec<u8>> {
            None
        }
        fn keychain_set(&self, _key: String, _value: Vec<u8>) -> bool {
            true
        }
        fn device_profile(&self) -> DeviceProfile {
            DeviceProfile {
                platform: Platform::Ios,
                model_identifier: "iPhone17,1".into(),
                device_name: String::new(),
                chip: "A18 Pro".into(),
                total_ram_bytes: 8 * 1024 * 1024 * 1024,
                app_memory_limit_bytes: 4_600_000_000,
                gpu_family: "metal3".into(),
                free_disk_bytes: 100 * 1024 * 1024 * 1024,
                thermal_state: ThermalState::Nominal,
                low_power_mode: false,
                has_increased_memory_entitlement: true,
            }
        }
        fn app_support_dir(&self) -> String {
            self.dir.clone()
        }
        fn now_ms(&self) -> i64 {
            1_760_400_000_000
        }
    }

    fn open_core() -> Arc<PocketCore> {
        let dir = tempfile::tempdir().unwrap().keep();
        let hooks = FakeHooks {
            dir: dir.to_string_lossy().to_string(),
        };
        PocketCore::open(
            CoreConfig {
                app_support_dir: hooks.dir.clone(),
                platform: Platform::Ios,
            },
            Arc::new(hooks),
        )
        .unwrap()
    }

    /// Marks `id` installed in the store so `is_model_runnable` accepts it and
    /// `send()` resolves the conversation's bound model instead of erroring
    /// with "No model is installed".
    fn install_model(core: &PocketCore, id: &str) {
        use pocket_store::repos::models::{ModelRow, ModelsRepo};
        ModelsRepo::new(core.store_for_test())
            .upsert(&ModelRow {
                id: id.into(),
                manifest_json: "{}".into(),
                state: "installed".into(),
                bytes_done: 0,
                installed_at: Some(1),
                last_used_at: None,
                pinned_engine: None,
                benchmark_json: None,
            })
            .unwrap();
    }

    #[test]
    fn catalog_returns_real_entries_with_fit_reports() {
        let core = open_core();
        let entries = core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        });
        assert!(
            entries.len() >= 20,
            "expected the real bundled catalog, got {}",
            entries.len()
        );
        assert!(entries
            .iter()
            .any(|e| e.manifest.tags.contains(&"recommended".to_string())));
    }

    #[test]
    fn catalog_filters_by_modality() {
        let core = open_core();
        let vision_only = core.catalog(CatalogFilter {
            modality: Some(Modality::Vision),
            curated_only: false,
            installed_only: false,
        });
        assert!(!vision_only.is_empty());
        assert!(vision_only
            .iter()
            .all(|e| e.manifest.modalities.contains(&Modality::Vision)));
    }

    #[test]
    fn create_list_and_delete_conversation_round_trips() {
        let core = open_core();
        let created = core
            .create_conversation(NewConversation {
                title: Some("Test chat".into()),
                project_id: None,
                model_id: None,
            })
            .unwrap();
        assert_eq!(created.title, Some("Test chat".into()));

        let list = core.conversations(ConversationQuery {
            search: None,
            archived: None,
            limit: 50,
            offset: 0,
        });
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, created.id);

        core.delete_conversation(created.id.clone()).unwrap();
        let after = core.conversations(ConversationQuery {
            search: None,
            archived: None,
            limit: 50,
            offset: 0,
        });
        assert!(after.is_empty());
    }

    #[test]
    fn audit_log_starts_empty() {
        let core = open_core();
        let entries = core.audit_log(AuditQuery {
            limit: 10,
            since_ts: None,
        });
        assert!(entries.is_empty());
    }

    #[test]
    fn devices_empty_then_revoke_errors_gracefully_on_missing_device() {
        let core = open_core();
        assert!(core.devices().is_empty());
        // Revoking a nonexistent device must not panic; repo update on 0 rows is Ok.
        assert!(core.revoke_device("nonexistent".into()).is_ok());
    }

    #[test]
    fn pairing_identity_needs_no_pro_and_does_not_rotate_secret() {
        let core = open_core();
        // No Pro: identity still readable (it carries no secret).
        let identity = core.pairing_identity();
        assert!(!identity.host_id.is_empty());
        assert!(!identity.cert_fingerprint.is_empty());

        // With Pro: reading identity after an offer must not invalidate the
        // offer's single-use secret (same fields, no rotation).
        core.activate_license(valid_test_license_key()).unwrap();
        let offer = core.pairing_offer().unwrap();
        let identity = core.pairing_identity();
        assert_eq!(identity.host_id, offer.host_id);
        assert_eq!(identity.host_name, offer.host_name);
        assert_eq!(identity.cert_fingerprint, offer.cert_fingerprint);
    }

    #[test]
    fn pairing_offer_requires_pro() {
        let core = open_core();
        let err = core.pairing_offer().unwrap_err();
        assert!(matches!(err, PocketError::NotEntitled { .. }));
    }

    #[test]
    fn pairing_offer_round_trips_through_parse_pairing_qr() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();

        let offer = core.pairing_offer().unwrap();
        assert!(!offer.cert_fingerprint.is_empty());
        assert!(!offer.secret.is_empty());

        // Build the QR URI the way Swift would render it, then parse it back
        // as the scanning phone would - proves pairing_offer and
        // parse_pairing_qr agree on the wire format (A§7).
        let uri = pocket_trust::build_pairing_uri(&pocket_trust::PairingUri {
            host_id: offer.host_id.clone(),
            host_name: offer.host_name.clone(),
            addrs: offer.addrs.clone(),
            port: offer.port,
            cert_fingerprint: offer.cert_fingerprint.clone(),
            secret: offer.secret.clone(),
        });
        let parsed = core.parse_pairing_qr(uri).unwrap();
        assert_eq!(parsed.host_id, offer.host_id);
        assert_eq!(parsed.cert_fingerprint, offer.cert_fingerprint);
        assert_eq!(parsed.secret, offer.secret);
    }

    #[test]
    fn pairing_offer_advertises_reachable_address_not_file_path() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();

        let offer = core.pairing_offer().unwrap();
        println!("Pairing offer addrs: {:?}", offer.addrs);
        assert!(!offer.addrs.is_empty(), "addrs must not be empty");
        for addr in &offer.addrs {
            assert!(
                !addr.contains('/'),
                "addr must be a valid host or IP, not a file path: {addr}"
            );
            assert!(
                !addr.contains("Application Support"),
                "addr must not contain app support dir: {addr}"
            );
            let is_ip = addr.parse::<std::net::Ipv4Addr>().is_ok();
            let is_bonjour = addr.ends_with(".local") && !addr.starts_with('.');
            assert!(
                is_ip || is_bonjour,
                "addr should be an IPv4 or Bonjour .local hostname, got: {addr}"
            );
        }
        assert_eq!(offer.cert_fingerprint, core.host_tls_identity().fingerprint_b64url);

        if let Ok(which) = std::process::Command::new("which").arg("openssl").output() {
            if which.status.success() {
                let dir = tempfile::tempdir().unwrap();
                let der_path = dir.path().join("cert.der");
                std::fs::write(&der_path, &core.host_tls_identity().cert_der).unwrap();
                let output = std::process::Command::new("openssl")
                    .args(["x509", "-inform", "der", "-in"])
                    .arg(&der_path)
                    .args(["-noout", "-text"])
                    .output()
                    .unwrap();
                let text = String::from_utf8_lossy(&output.stdout);
                for addr in &offer.addrs {
                    assert!(
                        text.contains(addr),
                        "cert SANs must include advertised address {addr}"
                    );
                }
            }
        }
    }

    fn urlencode_slash_at(s: &str) -> String {
        s.replace('/', "%2F").replace('@', "%40")
    }

    fn tls_client_config() -> Arc<rustls::ClientConfig> {
        static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
        CONFIG
            .get_or_init(|| {
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
                    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error>
                    {
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

                Arc::new(
                    rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(NoVerify))
                        .with_no_client_auth(),
                )
            })
            .clone()
    }

    fn open_tls_stream(port: u16) -> (rustls::ClientConnection, std::net::TcpStream) {
        let sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let server_name = rustls::pki_types::ServerName::try_from("127.0.0.1")
            .unwrap()
            .to_owned();
        let conn = rustls::ClientConnection::new(tls_client_config(), server_name).unwrap();
        (conn, sock)
    }

    /// Raw blocking TCP HTTP/1.1 (not reqwest - NET-15 restricts that crate
    /// to pocket-trust/pocket-models). Blocking (not async) on purpose: this
    /// test calls PocketCore methods that `block_on` their own dedicated
    /// runtime (D21/D30), which panics if called from inside a tokio
    /// runtime already driving the test itself.
    fn http_request(
        port: u16,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> (u16, serde_json::Value) {
        use std::io::{Read, Write};
        let (mut conn, mut sock) = open_tls_stream(port);
        let mut stream = rustls::Stream::new(&mut conn, &mut sock);
        let body_str = body.map(|b| b.to_string()).unwrap_or_default();
        let request = if body.is_some() {
            format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
                body_str.len()
            )
        } else {
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        };
        stream.write_all(request.as_bytes()).unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        let text = String::from_utf8_lossy(&buf);
        let status: u16 = text
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let json_body = text.split("\r\n\r\n").nth(1).unwrap_or("{}");
        (
            status,
            serde_json::from_str(json_body).unwrap_or(serde_json::Value::Null),
        )
    }

    #[test]
    fn control_api_catalog_and_status_and_delete_through_the_real_server() {
        let core = open_core();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let (catalog_status, catalog_body) =
            http_request(status.port, "GET", "/pocket/v1/catalog", None);
        assert_eq!(catalog_status, 200);
        assert!(catalog_body["entries"].as_array().unwrap().len() >= 20);

        let (status_status, status_body) =
            http_request(status.port, "GET", "/pocket/v1/status", None);
        assert_eq!(status_status, 200);
        assert!(status_body["server"]["running"].as_bool().unwrap());

        // Deleting a model that was never downloaded still succeeds (no-op
        // directory removal + a repo delete that affects zero rows).
        let entries = core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        });
        let id = entries[0].manifest.id.clone();
        // Model ids embed a '/' (repo path), so callers must percent-encode
        // them for a single {id} path segment - axum's Path extractor
        // percent-decodes automatically.
        let (delete_status, _) = http_request(
            status.port,
            "DELETE",
            &format!("/pocket/v1/models/{}", urlencode_slash_at(&id)),
            None,
        );
        assert_eq!(delete_status, 200);

        core.server_stop().unwrap();
    }

    #[test]
    fn control_api_download_without_a_coordinator_returns_500() {
        let core = open_core();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let entries = core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        });
        let id = entries[0].manifest.id.clone();

        let (download_status, body) = http_request(
            status.port,
            "POST",
            &format!("/pocket/v1/models/{}/download", urlencode_slash_at(&id)),
            None,
        );
        assert_eq!(download_status, 500);
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("DownloadCoordinator"));

        core.server_stop().unwrap();
    }

    #[test]
    fn full_pairing_flow_through_the_real_server() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let offer = core.pairing_offer().unwrap();

        let body = serde_json::json!({
            "secret": offer.secret,
            "device_name": "Rohit's iPhone",
            "device_pubkey": "abc",
            "platform": "ios",
        });
        let (status_code, create_body) =
            http_request(status.port, "POST", "/pocket/v1/pair", Some(&body));
        assert_eq!(status_code, 200);
        let pairing_id = create_body["pairing_id"].as_str().unwrap().to_string();

        core.approve_pairing(pairing_id.clone()).unwrap();

        let (poll_status, poll_body) = http_request(
            status.port,
            "GET",
            &format!("/pocket/v1/pair/{pairing_id}"),
            None,
        );
        assert_eq!(poll_status, 200);
        assert_eq!(poll_body["status"], "approved");
        assert!(poll_body["token"].is_string());

        // The approved device actually landed in storage.
        assert_eq!(core.devices().len(), 1);

        core.server_stop().unwrap();
    }

    /// NET-23: what the phone does first in the manual flow - an unpinned
    /// TLS GET /pocket/v1/hello - returning the base64url SHA-256 of the
    /// leaf cert the server actually presented.
    fn observed_fingerprint(port: u16) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use sha2::{Digest, Sha256};
        use std::io::{Read, Write};
        let (mut conn, mut sock) = open_tls_stream(port);
        {
            let mut stream = rustls::Stream::new(&mut conn, &mut sock);
            stream
                .write_all(b"GET /pocket/v1/hello HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf);
        }
        let leaf = conn.peer_certificates().unwrap()[0].as_ref().to_vec();
        URL_SAFE_NO_PAD.encode(Sha256::digest(leaf))
    }

    fn manual_pair_request(port: u16, code: &str) -> (u16, serde_json::Value) {
        let body = serde_json::json!({
            "code": code,
            "device_name": "Rohit's iPhone",
            "device_pubkey": "",
            "platform": "ios",
        });
        http_request(port, "POST", "/pocket/v1/pair", Some(&body))
    }

    /// NET-23 end to end over the real TLS server: manual code, unpinned
    /// connect, both sides' SAS agree, Allow, then the phone pins what it saw.
    #[test]
    fn full_manual_pairing_flow_through_the_real_server() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        // Mac: "Enter code manually".
        let manual = core.pairing_manual_code().unwrap();
        assert_eq!(manual.code.len(), 8);
        assert_eq!(manual.port, status.port);
        assert!(!manual.addrs.is_empty());

        // Phone: TOFU connect, record the cert, send the (typed) code.
        let seen_fp = observed_fingerprint(status.port);
        let typed = format!("{}-{}", &manual.code[..4], manual.code[4..].to_lowercase());
        let code = crate::normalize_pairing_code(typed).unwrap();
        let (status_code, create_body) = manual_pair_request(status.port, &code);
        assert_eq!(status_code, 200);
        let pairing_id = create_body["pairing_id"].as_str().unwrap().to_string();
        let phone_sas = crate::pairing_sas(seen_fp.clone(), code.clone()).unwrap();

        // Mac: the Allow sheet shows the same 6 digits.
        let pending = core.pending_pairings();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sas.as_deref(), Some(phone_sas.as_str()));
        // A man-in-the-middle's cert would have shown a different number.
        let mitm_fp = pocket_trust::generate_identity("mitm.local", &[]).fingerprint_b64url;
        assert_ne!(crate::pairing_sas(mitm_fp, code.clone()).unwrap(), phone_sas);

        core.approve_pairing(pairing_id.clone()).unwrap();
        let (_, poll_body) = http_request(
            status.port,
            "GET",
            &format!("/pocket/v1/pair/{pairing_id}"),
            None,
        );
        assert_eq!(poll_body["status"], "approved");
        assert!(poll_body["token"].is_string());
        assert_eq!(core.devices().len(), 1);

        // Phone: only now pin the cert it saw - identical to a QR pairing.
        let hello = http_request(status.port, "GET", "/pocket/v1/hello", None).1;
        let peer = core
            .save_peer(ApprovedPairing {
                host_id: hello["host_id"].as_str().unwrap().into(),
                host_name: poll_body["host_name"].as_str().unwrap().into(),
                cert_fingerprint: seen_fp.clone(),
                addr: "127.0.0.1".into(),
                port: status.port,
                device_id: "dev-manual".into(),
                token_keychain_ref: "peer.token.x".into(),
            })
            .unwrap();
        assert_eq!(peer.cert_sha256, core.pairing_identity().cert_fingerprint);

        // Single use: the same code can't start a second pairing.
        let (again, body) = manual_pair_request(status.port, &code);
        assert_eq!(again, 401);
        assert_eq!(body["error"]["code"], "no_code");

        core.server_stop().unwrap();
    }

    #[test]
    fn denied_manual_pairing_stores_nothing() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let manual = core.pairing_manual_code().unwrap();
        let (_, create_body) = manual_pair_request(status.port, &manual.code);
        let pairing_id = create_body["pairing_id"].as_str().unwrap().to_string();
        core.deny_pairing(pairing_id.clone()).unwrap();
        let (_, poll_body) = http_request(
            status.port,
            "GET",
            &format!("/pocket/v1/pair/{pairing_id}"),
            None,
        );
        assert_eq!(poll_body["status"], "denied");
        assert!(poll_body["token"].is_null());
        assert!(core.devices().is_empty());
        core.server_stop().unwrap();
    }

    #[test]
    fn manual_code_requires_pro_and_limits_tries() {
        let core = open_core();
        assert!(matches!(
            core.pairing_manual_code(),
            Err(PocketError::NotEntitled { .. })
        ));
        core.activate_license(valid_test_license_key()).unwrap();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let manual = core.pairing_manual_code().unwrap();
        for _ in 0..4 {
            assert_eq!(manual_pair_request(status.port, "ZZZZZZZZ").1["error"]["code"], "wrong_code");
        }
        assert_eq!(manual_pair_request(status.port, "ZZZZZZZZ").1["error"]["code"], "code_locked");
        assert_eq!(manual_pair_request(status.port, &manual.code).0, 401);
        // A new code starts fresh.
        let fresh = core.pairing_manual_code().unwrap();
        assert_eq!(manual_pair_request(status.port, &fresh.code).0, 200);
        core.server_stop().unwrap();
    }

    #[test]
    fn save_peer_then_peers_round_trips() {
        let core = open_core();
        let saved = core
            .save_peer(ApprovedPairing {
                host_id: "01JHOST".into(),
                host_name: "Rohit's MacBook Pro".into(),
                cert_fingerprint: "deadbeef".into(),
                addr: "192.168.1.5".into(),
                port: 7480,
                device_id: "dev1".into(),
                token_keychain_ref: "keychain:dev1".into(),
            })
            .unwrap();
        assert_eq!(saved.id, "dev1");

        let peers = core.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].addrs, vec!["192.168.1.5".to_string()]);
        assert_eq!(peers[0].name, "Rohit's MacBook Pro");
    }

    /// UI-24: an empty host_name (pre-UI-24 payload) stores the old host_id,
    /// but a path stored as a name must never surface in the UI — peers()
    /// degrades it to "Mac" until a re-pair stores a real name.
    #[test]
    fn peer_with_path_name_degrades_to_mac() {
        let core = open_core();
        core.save_peer(ApprovedPairing {
            host_id: "/Users/rohit/Library/Application Support/com.getpocketai.app".into(),
            host_name: String::new(),
            cert_fingerprint: "deadbeef".into(),
            addr: "192.168.1.5".into(),
            port: 7480,
            device_id: "dev2".into(),
            token_keychain_ref: "keychain:dev2".into(),
        })
        .unwrap();
        assert_eq!(core.peers()[0].name, "Mac");
    }

    /// MAC-21: re-pairing the same Mac replaces its row (keeping the id chats
    /// are bound to) instead of leaving a dead duplicate behind.
    #[test]
    fn repairing_same_mac_replaces_row_and_keeps_id() {
        let core = open_core();
        let pair = |fp: &str, dev: &str| ApprovedPairing {
            host_id: "01JHOST".into(),
            host_name: "Rohit's MacBook Pro".into(),
            cert_fingerprint: fp.into(),
            addr: "192.168.1.5".into(),
            port: 7480,
            device_id: dev.into(),
            token_keychain_ref: format!("peer.token.{dev}"),
        };
        core.save_peer(pair("old-cert", "dev1")).unwrap();
        let again = core.save_peer(pair("new-cert", "dev2")).unwrap();
        assert_eq!(again.id, "dev1");
        let peers = core.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].cert_sha256, "new-cert");
        assert_eq!(peers[0].token_ref, "peer.token.dev2");
    }

    /// MAC-21: host id is stable, short and not a path; the TLS identity (the
    /// phone's pin) survives a relaunch via the Keychain.
    #[test]
    fn host_id_and_cert_survive_relaunch() {
        struct MemHooks(FakeHooks, Mutex<std::collections::HashMap<String, Vec<u8>>>);
        impl PlatformHooks for MemHooks {
            fn keychain_get(&self, k: String) -> Option<Vec<u8>> {
                self.1.lock().unwrap().get(&k).cloned()
            }
            fn keychain_set(&self, k: String, v: Vec<u8>) -> bool {
                self.1.lock().unwrap().insert(k, v);
                true
            }
            fn device_profile(&self) -> DeviceProfile {
                self.0.device_profile()
            }
            fn app_support_dir(&self) -> String {
                self.0.app_support_dir()
            }
            fn now_ms(&self) -> i64 {
                self.0.now_ms()
            }
        }
        let dir = tempfile::tempdir()
            .unwrap()
            .keep()
            .to_string_lossy()
            .to_string();
        let hooks = Arc::new(MemHooks(FakeHooks { dir: dir.clone() }, Mutex::default()));
        let open = || {
            PocketCore::open(
                CoreConfig {
                    app_support_dir: dir.clone(),
                    platform: Platform::MacOs,
                },
                hooks.clone(),
            )
            .unwrap()
        };
        let first = open().pairing_identity();
        assert!(!first.host_id.contains('/'));
        assert!(first.host_id.len() < 63);
        let second = open().pairing_identity();
        assert_eq!(first.host_id, second.host_id);
        assert_eq!(first.cert_fingerprint, second.cert_fingerprint);
    }

    /// Real Keygen sample license key (LIC-01 / LIC-02), valid until 2028-03-31.
    fn valid_test_license_key() -> String {
        "key/eyJhY2NvdW50Ijp7ImlkIjoiNDBmZTcyNjAtZDc5Zi00NmFkLTljMGMtOTcyMzMxZjRmZmMwIn0sInByb2R1Y3QiOnsiaWQiOiIwYjc5MDQ1YS1kOTlkLTQwMjMtOWI4Ni02NjAwODA3YTliOGUifSwicG9saWN5Ijp7ImlkIjoiNDVhMGZmMGUtNzYzNy00M2FjLWIwNjMtODEzOTZlZDQ3MGU4IiwiZHVyYXRpb24iOm51bGx9LCJ1c2VyIjpudWxsLCJsaWNlbnNlIjp7ImlkIjoiMjNkNGFiNWItZTQzYy00YjAyLWE4ZWItYWRjNDcwNjgzOWY2IiwiY3JlYXRlZCI6IjIwMjYtMTAtMTVUMDY6NTg6MzcuMjAzWiIsImV4cGlyeSI6IjIwMjgtMDMtMzFUMTg6MzA6MDAuMDAwWiJ9fQ==.fhC7LYG5-OOt4G4mG34XETvcNXnRYNuMvx9kg-lktpuq9N7NACJndb1FVPYBYCRq-0CU-N8-eOi44w4gkkyLBA==".to_string()
    }

    #[test]
    fn is_offline_reflects_set_offline_mode() {
        let core = open_core();
        assert!(!core.is_offline());
        core.set_offline_mode(true);
        assert!(core.is_offline());
        core.set_offline_mode(false);
        assert!(!core.is_offline());
    }

    #[test]
    fn search_hub_refuses_while_offline() {
        let core = open_core();
        core.set_offline_mode(true);
        let err = core
            .search_hub("qwen3".into(), ModelFormat::MlxSafetensors)
            .unwrap_err();
        assert!(matches!(err, PocketError::Network { .. }));
    }

    #[test]
    fn server_start_free_tier_lan_refused_but_loopback_allowed() {
        let core = open_core();
        let err = core
            .server_start(ServerConfig {
                lan_enabled: true,
                port: 0,
            })
            .unwrap_err();
        assert!(matches!(err, PocketError::NotEntitled { .. }));

        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        assert!(status.running);
        assert!(!status.lan_enabled);
        core.server_stop().unwrap();
    }

    #[test]
    fn server_start_is_idempotent_and_status_reflects_it() {
        let core = open_core();
        assert!(!core.server_status().running);

        let first = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let second = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        assert_eq!(
            first.port, second.port,
            "idempotent start must not rebind a new port"
        );

        assert!(core.server_status().running);
        core.server_stop().unwrap();
        assert!(!core.server_status().running);
    }

    #[test]
    fn plan_download_then_verify_and_commit_round_trips_a_real_catalog_entry() {
        let core = open_core();
        let entries = core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        });
        let small_index = entries
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.manifest.files.iter().map(|f| f.size).sum::<u64>())
            .expect("catalog has entries")
            .0;
        let small = entries.into_iter().nth(small_index).unwrap();

        let plan = core.plan_download(small.manifest.id.clone()).unwrap();
        assert_eq!(
            plan.total_bytes,
            small.manifest.files.iter().map(|f| f.size).sum::<u64>()
        );
        assert!(!plan.files.is_empty());

        // Write the expected bytes to each .part so verify_and_commit succeeds
        // against the real planned paths (not a mocked plan).
        for (planned, file) in plan.files.iter().zip(&small.manifest.files) {
            std::fs::create_dir_all(std::path::Path::new(&planned.dest_part).parent().unwrap())
                .unwrap();
            // We don't have the real file bytes here, so write bytes that
            // hash to a known value and treat a checksum-mismatch path as
            // equally valid evidence that verify_and_commit is wired for real.
            std::fs::write(&planned.dest_part, b"not the real content").unwrap();
            let _ = &file.sha256;
        }

        match core.verify_and_commit(small.manifest.id.clone()) {
            Ok(()) => panic!("expected a checksum mismatch since we wrote placeholder bytes"),
            Err(PocketError::Storage { message }) => {
                assert!(message.contains("checksum"), "{message}")
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }

        core.delete_model(small.manifest.id.clone()).unwrap();
    }

    #[test]
    fn plan_download_refuses_when_disk_is_full() {
        struct NoDiskHooks(FakeHooks);
        impl PlatformHooks for NoDiskHooks {
            fn keychain_get(&self, k: String) -> Option<Vec<u8>> {
                self.0.keychain_get(k)
            }
            fn keychain_set(&self, k: String, v: Vec<u8>) -> bool {
                self.0.keychain_set(k, v)
            }
            fn device_profile(&self) -> DeviceProfile {
                let mut p = self.0.device_profile();
                p.free_disk_bytes = 0;
                p
            }
            fn app_support_dir(&self) -> String {
                self.0.app_support_dir()
            }
            fn now_ms(&self) -> i64 {
                self.0.now_ms()
            }
        }

        let dir = tempfile::tempdir().unwrap().keep();
        let hooks = NoDiskHooks(FakeHooks {
            dir: dir.to_string_lossy().to_string(),
        });
        let core = PocketCore::open(
            CoreConfig {
                app_support_dir: dir.to_string_lossy().to_string(),
                platform: Platform::Ios,
            },
            Arc::new(hooks),
        )
        .unwrap();

        let entries = core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        });
        let any = entries.first().unwrap();
        let err = core.plan_download(any.manifest.id.clone()).unwrap_err();
        assert!(matches!(err, PocketError::WillNotFit { .. }));
    }

    #[test]
    fn stop_everything_stops_the_server() {
        let core = open_core();
        core.server_start(ServerConfig {
            lan_enabled: false,
            port: 0,
        })
        .unwrap();
        assert!(core.server_status().running);
        core.stop_everything();
        assert!(!core.server_status().running);
    }

    /// Bridges the FFI-local `Engine` trait to a real `pocket_engine::MockEngine`,
    /// proving `send()`'s whole path (persist user message -> resolve model ->
    /// load -> generate -> accumulate -> persist+return assistant message)
    /// end to end - not a real `pocket_engine::Engine` adapter (that's D24's
    /// remaining work), just enough to drive `send()` for real.
    struct MockEngineBridge(pocket_engine::MockEngine);

    impl PocketKitEngine for MockEngineBridge {
        fn kind(&self) -> EngineKind {
            EngineKind::Mlx
        }
        fn supports(&self, _model: ModelManifest) -> bool {
            true
        }
        fn load(
            &self,
            _model: ModelManifest,
            _opts: LoadOptions,
        ) -> Result<LoadedModel, PocketError> {
            Ok(LoadedModel {
                handle: "bridge".into(),
            })
        }
        fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        fn generate(
            &self,
            handle: String,
            req: GenerateRequest,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            use pocket_engine::Engine as _;
            let pocket_req = pocket_types::GenerateRequest {
                request_id: req.request_id,
                messages: req
                    .messages
                    .into_iter()
                    .map(|m| pocket_types::ChatMessage {
                        role: m.role,
                        parts: m
                            .parts
                            .into_iter()
                            .map(|p| match p {
                                ContentPart::Text { text } => {
                                    pocket_types::ContentPart::Text { text }
                                }
                                ContentPart::ImageRef { path } => {
                                    pocket_types::ContentPart::ImageRef { path }
                                }
                            })
                            .collect(),
                    })
                    .collect(),
                tools: vec![],
                params: pocket_types::SamplingParams {
                    temperature: req.params.temperature,
                    top_p: req.params.top_p,
                    top_k: req.params.top_k,
                    min_p: req.params.min_p,
                    max_tokens: req.params.max_tokens,
                    stop: req.params.stop,
                    seed: req.params.seed,
                    repetition_penalty: req.params.repetition_penalty,
                },
                reasoning: pocket_types::ReasoningMode::Off,
            };
            struct SinkBridge(Arc<dyn PocketKitTokenSink>);
            impl pocket_engine::TokenSink for SinkBridge {
                fn on_delta(&self, delta: pocket_types::GenerateDelta) {
                    self.0.on_delta(GenerateDelta {
                        text: delta.text,
                        reasoning: delta.reasoning,
                        tool_calls: vec![],
                    });
                }
                fn on_done(&self, stats: pocket_types::GenerateStats) {
                    self.0.on_done(GenerateStats {
                        prompt_tokens: stats.prompt_tokens,
                        completion_tokens: stats.completion_tokens,
                        ttft_ms: stats.ttft_ms,
                        decode_tps: stats.decode_tps,
                        finish: FinishReason::Stop,
                    });
                }
                fn on_error(&self, _err: pocket_types::PocketError) {}
            }
            let rt = tokio::runtime::Runtime::new().unwrap();
            let stats = rt
                .block_on(
                    self.0
                        .generate(handle, pocket_req, Arc::new(SinkBridge(sink))),
                )
                .map_err(|e| PocketError::EngineFailed {
                    engine: EngineKind::Mlx,
                    message: e.to_string(),
                })?;
            Ok(GenerateStats {
                prompt_tokens: stats.prompt_tokens,
                completion_tokens: stats.completion_tokens,
                ttft_ms: stats.ttft_ms,
                decode_tps: stats.decode_tps,
                finish: FinishReason::Stop,
            })
        }
        fn embed(
            &self,
            handle: String,
            inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            use pocket_engine::Engine as _;
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(self.0.embed(handle, inputs))
                .map_err(|e| PocketError::EngineFailed {
                    engine: EngineKind::Mlx,
                    message: e.to_string(),
                })
        }
        fn cancel(&self, _request_id: String) {}
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    struct CollectingChatSink {
        deltas: Mutex<Vec<String>>,
        saved: Mutex<Option<Message>>,
    }

    impl ChatSink for CollectingChatSink {
        fn on_delta(&self, delta: GenerateDelta) {
            self.deltas.lock().unwrap().push(delta.text);
        }
        fn on_done(&self, _stats: GenerateStats) {}
        fn on_error(&self, _err: PocketError) {}
        fn on_message_saved(&self, message: Message) {
            *self.saved.lock().unwrap() = Some(message);
        }
        fn on_tool_step(&self, _step: ToolStep) {}
    }

    #[test]
    fn send_drives_a_full_chat_turn_through_a_registered_engine() {
        let core = open_core();
        core.register_engine(Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(
            2000.0,
        ))));

        let model_id = "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e";
        install_model(&core, model_id);
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Test".into()),
                project_id: None,
                model_id: Some(model_id.into()),
            })
            .unwrap();

        let sink = Arc::new(CollectingChatSink {
            deltas: Mutex::new(vec![]),
            saved: Mutex::new(None),
        });
        let message = core
            .send(
                conversation.id.clone(),
                UserInput {
                    parts: vec![ContentPart::Text {
                        text: "Hello!".into(),
                    }],
                },
                sink.clone(),
            )
            .unwrap();

        assert_eq!(message.role, "assistant");
        let ContentPart::Text { text } = &message.parts[0] else {
            panic!("expected text")
        };
        assert!(!text.is_empty());
        assert!(sink.saved.lock().unwrap().is_some());
        assert!(!sink.deltas.lock().unwrap().is_empty());

        // Both the user and assistant turns persisted.
        assert_eq!(core.messages(conversation.id).len(), 2);
    }

    /// AGT-12: first reply calls fetch_url (a non-http URL, so no network),
    /// then answers. Records whether each request carried tools.
    struct ToolCallingEngine {
        calls: std::sync::atomic::AtomicUsize,
        tools_seen: Mutex<Vec<usize>>,
    }

    impl PocketKitEngine for ToolCallingEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::Mlx
        }
        fn supports(&self, _model: ModelManifest) -> bool {
            true
        }
        fn load(&self, _m: ModelManifest, _o: LoadOptions) -> Result<LoadedModel, PocketError> {
            Ok(LoadedModel {
                handle: "tools".into(),
            })
        }
        fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        fn generate(
            &self,
            _handle: String,
            req: GenerateRequest,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            self.tools_seen.lock().unwrap().push(req.tools.len());
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let after_tool = req.messages.last().is_some_and(|m| m.role == "tool");
            let text = if !after_tool && !req.tools.is_empty() {
                r#"<tool_call>{"name": "fetch_url", "arguments": {"url": "ftp://example.com/x"}}</tool_call>"#
            } else {
                "Done."
            };
            sink.on_delta(GenerateDelta {
                text: text.into(),
                reasoning: String::new(),
                tool_calls: vec![],
            });
            let stats = GenerateStats {
                prompt_tokens: 1,
                completion_tokens: 1,
                ttft_ms: 0,
                decode_tps: 0.0,
                finish: FinishReason::Stop,
            };
            sink.on_done(stats.clone());
            Ok(stats)
        }
        fn embed(&self, _h: String, _i: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(vec![])
        }
        fn cancel(&self, _request_id: String) {}
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    #[test]
    fn tools_switch_runs_the_tool_loop_and_persists_steps() {
        let core = open_core();
        let engine = Arc::new(ToolCallingEngine {
            calls: Default::default(),
            tools_seen: Mutex::new(vec![]),
        });
        core.register_engine(engine.clone());
        let model_id = "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e";
        install_model(&core, model_id);
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Test".into()),
                project_id: None,
                model_id: Some(model_id.into()),
            })
            .unwrap();
        let ask = || UserInput {
            parts: vec![ContentPart::Text { text: "hi".into() }],
        };
        let sink = || {
            Arc::new(CollectingChatSink {
                deltas: Mutex::new(vec![]),
                saved: Mutex::new(None),
            })
        };

        // Off by default: no tools sent.
        assert!(!core.conversation_tools(conversation.id.clone()));
        core.send(conversation.id.clone(), ask(), sink()).unwrap();
        // (the first exchange also runs a tools-free title request)
        assert!(engine.tools_seen.lock().unwrap().iter().all(|&n| n == 0));

        core.set_conversation_tools(conversation.id.clone(), true).unwrap();
        assert!(core.conversation_tools(conversation.id.clone()));
        let reply = core.send(conversation.id.clone(), ask(), sink()).unwrap();
        let with_tools = engine.tools_seen.lock().unwrap().iter().filter(|&&n| n == 4).count();
        assert_eq!(with_tools, 2, "tool round + answer round");
        assert_eq!(reply.tool_steps.len(), 1);
        assert_eq!(reply.tool_steps[0].kind, "read");
        assert!(reply.sources.is_empty(), "a failed read is no source");

        // Steps survive a reload.
        let reloaded = core.messages(conversation.id.clone()).pop().unwrap();
        assert_eq!(reloaded.tool_steps, reply.tool_steps);
        let ContentPart::Text { text } = &reloaded.parts[0] else {
            panic!("expected text")
        };
        assert_eq!(text, "Done.");

        // Offline: tools on, but none sent.
        core.set_offline_mode(true);
        core.send(conversation.id.clone(), ask(), sink()).unwrap();
        assert_eq!(engine.tools_seen.lock().unwrap().last(), Some(&0));
    }

    fn dummy_manifest(format: ModelFormat) -> ModelManifest {
        ModelManifest {
            id: "test/model@rev".into(),
            display_name: "Test Model".into(),
            family: "Test".into(),
            params_b: 0.1,
            quant: "q4_k_m".into(),
            format,
            modalities: vec![Modality::Text],
            context_length: 2048,
            files: vec![ModelFile {
                path: match format {
                    ModelFormat::Gguf => "model.gguf".into(),
                    _ => "model.safetensors".into(),
                },
                size: 1024,
                sha256: "dummy".into(),
                url: "http://example.com/model".into(),
            }],
            license: LicenseInfo {
                spdx: "MIT".into(),
                commercial_ok: true,
                url: "http://example.com/license".into(),
            },
            hf_repo: "test/model".into(),
            hf_revision: "rev".into(),
            min_ram_bytes: 1024,
            est_runtime_bytes: 1024,
            tags: vec![],
            curated: false,
            reasoning: false,
        }
    }

    struct RejectAllEngine;

    impl PocketKitEngine for RejectAllEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::Mlx
        }
        fn supports(&self, _model: ModelManifest) -> bool {
            false
        }
        fn load(
            &self,
            _model: ModelManifest,
            _opts: LoadOptions,
        ) -> Result<LoadedModel, PocketError> {
            Err(PocketError::EngineFailed {
                engine: EngineKind::Mlx,
                message: "RejectAllEngine does not support loading".into(),
            })
        }
        fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        fn generate(
            &self,
            _handle: String,
            _req: GenerateRequest,
            _sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            Err(PocketError::EngineFailed {
                engine: EngineKind::Mlx,
                message: "RejectAllEngine cannot generate".into(),
            })
        }
        fn embed(
            &self,
            _handle: String,
            _inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(vec![])
        }
        fn cancel(&self, _request_id: String) {}
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    #[test]
    fn routing_prefers_supporting_registered_engine_even_for_gguf() {
        let core = open_core();
        let registered = Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(2000.0)));
        core.register_engine(registered);

        let gguf_manifest = dummy_manifest(ModelFormat::Gguf);
        let chosen = core.pick_engine(&gguf_manifest).unwrap();
        // Registered engine supports everything and takes precedence over built-in llama
        assert_eq!(chosen.kind(), EngineKind::Mlx);
    }

    #[test]
    fn routing_falls_back_to_llama_when_registered_engine_does_not_support_gguf() {
        let core = open_core();
        core.register_engine(Arc::new(RejectAllEngine));

        let gguf_manifest = dummy_manifest(ModelFormat::Gguf);
        let chosen = core.pick_engine(&gguf_manifest).unwrap();
        assert_eq!(chosen.kind(), EngineKind::LlamaCpp);

        // Also verify send() with GGUF attempts llama load instead of failing with "no Engine registered"
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("GGUF Chat".into()),
                project_id: None,
                model_id: Some(
                    "unsloth/Qwen3-4B-Instruct-2507-GGUF@a06e946bb6b655725eafa393f4a9745d460374c9"
                        .into(),
                ),
            })
            .unwrap();
        let sink = Arc::new(CollectingChatSink {
            deltas: Mutex::new(vec![]),
            saved: Mutex::new(None),
        });
        let err = core
            .send(
                conversation.id,
                UserInput {
                    parts: vec![ContentPart::Text { text: "Hi".into() }],
                },
                sink,
            )
            .unwrap_err();

        // Must not be "no Engine registered" or "no engine supports", it routes to LlamaCpp and attempts load
        match err {
            PocketError::EngineFailed { engine, message } => {
                assert_eq!(engine, EngineKind::LlamaCpp);
                assert!(!message.contains("no engine supports"));
                assert!(!message.contains("no Engine registered"));
            }
            PocketError::InvalidInput { .. } => {}
            other => panic!(
                "expected LlamaCpp EngineFailed or InvalidInput, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn routing_chooses_llama_when_no_registered_engine_for_gguf() {
        let core = open_core();
        let gguf_manifest = dummy_manifest(ModelFormat::Gguf);
        let chosen = core.pick_engine(&gguf_manifest).unwrap();
        assert_eq!(chosen.kind(), EngineKind::LlamaCpp);

        // Also test through send()
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("GGUF Chat 2".into()),
                project_id: None,
                model_id: Some(
                    "unsloth/Qwen3-4B-Instruct-2507-GGUF@a06e946bb6b655725eafa393f4a9745d460374c9"
                        .into(),
                ),
            })
            .unwrap();
        let sink = Arc::new(CollectingChatSink {
            deltas: Mutex::new(vec![]),
            saved: Mutex::new(None),
        });
        let err = core
            .send(
                conversation.id,
                UserInput {
                    parts: vec![ContentPart::Text { text: "Hi".into() }],
                },
                sink,
            )
            .unwrap_err();

        match err {
            PocketError::EngineFailed { engine, message } => {
                assert_eq!(engine, EngineKind::LlamaCpp);
                assert!(!message.contains("no engine supports"));
                assert!(!message.contains("no Engine registered"));
            }
            PocketError::InvalidInput { .. } => {}
            other => panic!(
                "expected LlamaCpp EngineFailed or InvalidInput, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn routing_errors_when_no_engine_supports_non_gguf() {
        let core = open_core();
        let mlx_manifest = dummy_manifest(ModelFormat::MlxSafetensors);
        let err = match core.pick_engine(&mlx_manifest) {
            Ok(_) => panic!("expected error for non-gguf manifest with no registered engine"),
            Err(e) => e,
        };
        match err {
            PocketError::EngineFailed { engine, message } => {
                assert_eq!(engine, EngineKind::Mlx);
                assert!(message.contains("no engine supports model test/model@rev"));
            }
            other => panic!("expected EngineFailed, got {:?}", other),
        }

        // Also test through send() with an installed non-GGUF model
        let model_id = "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e";
        install_model(&core, model_id);
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Non-GGUF Chat".into()),
                project_id: None,
                model_id: Some(model_id.into()),
            })
            .unwrap();
        let sink = Arc::new(CollectingChatSink {
            deltas: Mutex::new(vec![]),
            saved: Mutex::new(None),
        });
        let err = core
            .send(
                conversation.id,
                UserInput {
                    parts: vec![ContentPart::Text { text: "Hi".into() }],
                },
                sink,
            )
            .unwrap_err();
        match err {
            PocketError::EngineFailed { message, .. } => {
                assert!(message.contains("no engine supports model"));
            }
            other => panic!("expected EngineFailed, got {:?}", other),
        }
    }

    #[test]
    fn pocket_engine_adapter_delegation() {
        use crate::engine_adapter::PocketEngineAdapter;
        let mock = pocket_engine::MockEngine::new(2000.0);
        let adapter = PocketEngineAdapter::new(mock);
        assert_eq!(adapter.kind(), EngineKind::Mlx);
        let manifest = dummy_manifest(ModelFormat::MlxSafetensors);
        assert!(adapter.supports(manifest.clone()));
        let loaded = adapter.load(manifest, LoadOptions::default()).unwrap();
        assert!(!loaded.handle.is_empty());
        assert_eq!(
            adapter.resident_bytes(loaded.handle.clone()),
            256 * 1024 * 1024
        );
        adapter.cancel("req-1".into());
        adapter.unload(loaded.handle).unwrap();
    }

    #[test]
    fn find_manifest_resolves_mac_prefix_and_preserves_remote_id() {
        let core = open_core();
        let remote_id = "mac:my-mac/mlx-community/Qwen2.5-0.5B-Instruct-4bit@main";
        let manifest = core.find_manifest(&remote_id.to_string()).expect("remote model manifest should be found");
        assert_eq!(manifest.id, remote_id);
    }

    /// Same raw-socket approach as `http_request`, but returns the full
    /// response text unparsed - needed for SSE, which isn't a single JSON body.
    fn http_request_raw(
        port: u16,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> (u16, String) {
        use std::io::{Read, Write};
        let (mut conn, mut sock) = open_tls_stream(port);
        let mut stream = rustls::Stream::new(&mut conn, &mut sock);
        let body_str = body.map(|b| b.to_string()).unwrap_or_default();
        let request = if body.is_some() {
            format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
                body_str.len()
            )
        } else {
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        };
        stream.write_all(request.as_bytes()).unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
        let status: u16 = text
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let (head, raw_body) = text.split_once("\r\n\r\n").unwrap();
        let body_text = if head.to_lowercase().contains("transfer-encoding: chunked") {
            dechunk(raw_body)
        } else {
            raw_body.to_string()
        };
        (status, body_text)
    }

    /// Undoes HTTP/1.1 chunked transfer encoding (axum's SSE response streams
    /// as chunks, so a raw-socket reader sees `<hex size>\r\n<data>\r\n...`
    /// rather than one contiguous body).
    fn dechunk(body: &str) -> String {
        let mut out = String::new();
        let mut rest = body;
        while let Some((size_line, after)) = rest.split_once("\r\n") {
            let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            out.push_str(&after[..size.min(after.len())]);
            rest = after.get(size + 2..).unwrap_or("");
        }
        out
    }

    fn open_core_with_mock_engine() -> Arc<PocketCore> {
        let core = open_core();
        core.register_engine(Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(
            2000.0,
        ))));
        core
    }

    fn first_catalog_model_id(core: &PocketCore) -> String {
        core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        })[0]
            .manifest
            .id
            .clone()
    }

    /// NET-20: a generation whose device was released before it started
    /// never reaches the engine.
    #[test]
    fn generate_chat_skips_a_dropped_request() {
        struct CountingSink(std::sync::atomic::AtomicUsize);
        impl crate::traits::TokenSink for CountingSink {
            fn on_delta(&self, _d: GenerateDelta) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            fn on_done(&self, _s: GenerateStats) {}
            fn on_error(&self, _e: PocketError) {}
        }
        let core = open_core_with_mock_engine();
        let model = first_catalog_model_id(&core);
        let sink = Arc::new(CountingSink(Default::default()));
        let dropped = std::sync::atomic::AtomicBool::new(true);
        let msg = || {
            vec![ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text { text: "hi".into() }],
            }]
        };
        let params = || SamplingParams {
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.0,
            max_tokens: 16,
            stop: vec![],
            seed: None,
            repetition_penalty: 1.1,
        };
        let r = core.generate_chat_for("r1".into(), &model, msg(), params(), sink.clone(), Some(&dropped));
        assert!(matches!(r, Err(PocketError::Cancelled)));
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        dropped.store(false, std::sync::atomic::Ordering::SeqCst);
        core.generate_chat_for("r2".into(), &model, msg(), params(), sink.clone(), Some(&dropped))
            .unwrap();
        assert!(sink.0.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[test]
    fn chat_completions_nonstream_matches_openai_shape() {
        let core = open_core_with_mock_engine();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let model = first_catalog_model_id(&core);

        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Say hello in five words."}],
            "temperature": 0.7,
            "stream": false,
        });
        let (status_code, resp) =
            http_request(status.port, "POST", "/v1/chat/completions", Some(&body));
        assert_eq!(status_code, 200);
        assert_eq!(resp["object"], "chat.completion");
        assert_eq!(resp["model"], model);
        assert_eq!(resp["choices"][0]["message"]["role"], "assistant");
        assert!(!resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(resp["choices"][0]["finish_reason"], "stop");
        assert!(resp["usage"]["total_tokens"].as_u64().unwrap() > 0);

        core.server_stop().unwrap();
    }

    #[test]
    fn chat_completions_stream_emits_role_chunk_content_chunks_and_done() {
        let core = open_core_with_mock_engine();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let model = first_catalog_model_id(&core);

        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Say hello in five words."}],
            "stream": true,
        });
        let (status_code, text) =
            http_request_raw(status.port, "POST", "/v1/chat/completions", Some(&body));
        assert_eq!(status_code, 200);

        let chunks: Vec<&str> = text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| s.starts_with("data:"))
            .collect();
        assert!(chunks.last().unwrap().contains("[DONE]"));

        let role_chunk: serde_json::Value =
            serde_json::from_str(chunks[0].trim_start_matches("data:").trim()).unwrap();
        assert_eq!(role_chunk["object"], "chat.completion.chunk");
        assert_eq!(role_chunk["choices"][0]["delta"]["role"], "assistant");

        let parsed: Vec<serde_json::Value> = chunks[1..chunks.len() - 1]
            .iter()
            .map(|c| serde_json::from_str(c.trim_start_matches("data:").trim()).unwrap())
            .collect();
        assert!(parsed
            .iter()
            .any(|c| c["choices"][0]["finish_reason"] == "stop"));
        let usage_chunk = parsed
            .iter()
            .find(|c| c["choices"].as_array().unwrap().is_empty())
            .expect("expected a usage-only chunk with an empty choices array");
        assert!(usage_chunk["usage"]["total_tokens"].as_u64().unwrap() > 0);

        core.server_stop().unwrap();
    }

    #[test]
    fn chat_completions_rejects_remote_image_url() {
        let core = open_core_with_mock_engine();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let model = first_catalog_model_id(&core);

        let body = serde_json::json!({
            "model": model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "what is this?"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/cat.png"}},
                ],
            }],
            "stream": false,
        });
        let (status_code, resp) =
            http_request(status.port, "POST", "/v1/chat/completions", Some(&body));
        assert_eq!(status_code, 400);
        assert_eq!(resp["error"]["code"], "unsupported_image_url");

        core.server_stop().unwrap();
    }

    #[test]
    fn chat_completions_404s_for_unknown_model() {
        let core = open_core_with_mock_engine();
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let body = serde_json::json!({
            "model": "nonexistent/model",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false,
        });
        let (status_code, resp) =
            http_request(status.port, "POST", "/v1/chat/completions", Some(&body));
        assert_eq!(status_code, 404);
        assert_eq!(resp["error"]["code"], "model_not_found");

        core.server_stop().unwrap();
    }

    #[test]
    fn chat_completions_feed_the_request_log_and_stats() {
        let core = open_core_with_mock_engine();
        assert_eq!(core.server_stats().requests_today, 0);
        assert!(core.server_request_log(200).is_empty());
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        let model = first_catalog_model_id(&core);
        let chat = |model: &str, stream: bool| {
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "Say hello in five words."}],
                "stream": stream,
            })
        };

        let (code, resp) =
            http_request(status.port, "POST", "/v1/chat/completions", Some(&chat(&model, false)));
        assert_eq!(code, 200);
        let (code, _) =
            http_request_raw(status.port, "POST", "/v1/chat/completions", Some(&chat(&model, true)));
        assert_eq!(code, 200);
        let (code, _) =
            http_request(status.port, "POST", "/v1/chat/completions", Some(&chat("nope/x", false)));
        assert_eq!(code, 404);

        let log = core.server_request_log(200);
        assert_eq!(log.len(), 3);
        // Newest first: the 404, then the stream, then the non-stream call.
        assert_eq!(log.iter().map(|e| e.status).collect::<Vec<_>>(), [404, 200, 200]);
        assert_eq!(log[0].model, "nope/x");
        assert_eq!(log[0].completion_tokens, 0);
        for e in &log {
            assert_eq!(e.client, "local"); // loopback, no API key
            assert_eq!(e.endpoint, "/v1/chat/completions");
        }
        let nonstream_tokens = resp["usage"]["completion_tokens"].as_u64().unwrap();
        assert_eq!(log[2].completion_tokens as u64, nonstream_tokens);
        assert!(log[1].completion_tokens > 0, "stream counted from final stats");
        assert_eq!(core.server_request_log(1).len(), 1);

        let stats = core.server_stats();
        assert_eq!(stats.requests_today, 3);
        assert_eq!(
            stats.tokens_out_today,
            (log[1].completion_tokens + log[2].completion_tokens) as u64
        );
        assert!(stats.avg_tokens_per_sec >= 0.0);

        core.server_stop().unwrap();
    }

    // ---- NET-08: audio + image media routes ----

    struct MockSttEngine;
    impl SpeechToTextEngine for MockSttEngine {
        fn transcribe(
            &self,
            _audio_path: String,
            _lang: Option<String>,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<(), PocketError> {
            sink.on_delta(GenerateDelta {
                text: "Hello, this is a test transcription.".into(),
                reasoning: String::new(),
                tool_calls: vec![],
            });
            sink.on_done(GenerateStats {
                prompt_tokens: 0,
                completion_tokens: 8,
                ttft_ms: 10,
                decode_tps: 100.0,
                finish: FinishReason::Stop,
            });
            Ok(())
        }
    }

    struct MockTtsEngine;
    impl TextToSpeechEngine for MockTtsEngine {
        fn synthesize(
            &self,
            _text: String,
            _voice: String,
            out_path: String,
        ) -> Result<(), PocketError> {
            std::fs::write(out_path, b"RIFF\x24\x00\x00\x00WAVEfmt fake-wav-bytes").map_err(|e| {
                PocketError::Storage {
                    message: e.to_string(),
                }
            })
        }
    }

    const MOCK_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    struct MockImageEngine;
    impl ImageEngine for MockImageEngine {
        fn generate(
            &self,
            _prompt: String,
            _negative: String,
            _size: String,
            _steps: u32,
            _seed: i64,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<(), PocketError> {
            sink.on_delta(GenerateDelta {
                text: MOCK_PNG_B64.into(),
                reasoning: String::new(),
                tool_calls: vec![],
            });
            sink.on_done(GenerateStats {
                prompt_tokens: 0,
                completion_tokens: 0,
                ttft_ms: 0,
                decode_tps: 0.0,
                finish: FinishReason::Stop,
            });
            Ok(())
        }
    }

    /// Raw blocking TCP send/receive that works for binary bodies (the JSON
    /// helpers above assume a UTF-8 body) - connects, writes `request` as-is,
    /// reads to EOF, splits headers from body, and dechunks if needed.
    fn raw_http(port: u16, request: &[u8]) -> (u16, Vec<u8>) {
        use std::io::{Read, Write};
        let (mut conn, mut sock) = open_tls_stream(port);
        let mut stream = rustls::Stream::new(&mut conn, &mut sock);
        stream.write_all(request).unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        let split_at = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let (head, body) = buf.split_at(split_at);
        let head_text = String::from_utf8_lossy(head);
        let status: u16 = head_text
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let body = if head_text
            .to_lowercase()
            .contains("transfer-encoding: chunked")
        {
            dechunk_bytes(body)
        } else {
            body.to_vec()
        };
        (status, body)
    }

    fn dechunk_bytes(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut rest = body;
        while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
            let (size_line, after) = rest.split_at(pos);
            let after = &after[2..];
            let size = usize::from_str_radix(std::str::from_utf8(size_line).unwrap().trim(), 16)
                .unwrap_or(0);
            if size == 0 {
                break;
            }
            out.extend_from_slice(&after[..size.min(after.len())]);
            rest = after.get(size + 2..).unwrap_or(&[]);
        }
        out
    }

    fn multipart_body(
        boundary: &str,
        fields: &[(&str, &str)],
        file_field: &str,
        filename: &str,
        file_bytes: &[u8],
    ) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, value) in fields {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        }
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(file_bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    #[test]
    fn transcribe_through_the_real_server_matches_golden_fixture() {
        let core = open_core();
        core.register_stt(Arc::new(MockSttEngine));
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let boundary = "pockettestboundary";
        let body = multipart_body(
            boundary,
            &[("model", "whisper-mock"), ("response_format", "json")],
            "file",
            "clip.wav",
            b"fake-wav-audio-bytes",
        );
        let request = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let mut full = request.into_bytes();
        full.extend_from_slice(&body);

        let (status_code, resp_body) = raw_http(status.port, &full);
        assert_eq!(status_code, 200);
        let actual: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/openai/audio.transcriptions.res.json"
        ))
        .unwrap();
        assert_eq!(actual["text"], fixture["text"]);

        core.server_stop().unwrap();
    }

    #[test]
    fn speech_through_the_real_server_returns_wav_bytes() {
        let core = open_core();
        core.register_tts(Arc::new(MockTtsEngine));
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let body = serde_json::json!({ "model": "tts-mock", "input": "hello", "voice": "alloy" });
        let (status_code, resp_body) =
            http_request_raw(status.port, "POST", "/v1/audio/speech", Some(&body));
        assert_eq!(status_code, 200);
        assert!(resp_body.contains("RIFF"));

        core.server_stop().unwrap();
    }

    #[test]
    fn images_generations_through_the_real_server_matches_golden_shape() {
        let core = open_core();
        core.register_image(Arc::new(MockImageEngine));
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let req_fixture = include_str!("../../../contracts/openai/images.generations.req.json");
        let body: serde_json::Value = serde_json::from_str(req_fixture).unwrap();
        let (status_code, resp) =
            http_request(status.port, "POST", "/v1/images/generations", Some(&body));
        assert_eq!(status_code, 200);
        assert_eq!(resp["data"][0]["b64_json"], MOCK_PNG_B64);
        assert!(resp["created"].as_i64().unwrap() > 0);

        core.server_stop().unwrap();
    }

    #[test]
    fn images_generations_rejects_n_over_four_and_unsupported_size() {
        let core = open_core();
        core.register_image(Arc::new(MockImageEngine));
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();

        let (too_many, _) = http_request(
            status.port,
            "POST",
            "/v1/images/generations",
            Some(&serde_json::json!({ "prompt": "x", "n": 5 })),
        );
        assert_eq!(too_many, 400);

        let (bad_size, _) = http_request(
            status.port,
            "POST",
            "/v1/images/generations",
            Some(&serde_json::json!({ "prompt": "x", "size": "99x99" })),
        );
        assert_eq!(bad_size, 400);

        core.server_stop().unwrap();
    }

    // ---- LIC-03: entitlements merge ----

    #[test]
    fn neither_source_active_is_not_pro() {
        let core = open_core();
        assert!(!core.entitlements().pro);
    }

    #[test]
    fn license_only_grants_pro() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();
        assert!(core.entitlements().pro);
    }

    #[test]
    fn store_purchase_only_grants_pro() {
        let core = open_core();
        core.set_store_entitlement(StoreEntitlement {
            pro: true,
            transaction_id: Some("txn1".into()),
        });
        assert!(core.entitlements().pro);
    }

    #[test]
    fn both_sources_active_is_pro() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();
        core.set_store_entitlement(StoreEntitlement {
            pro: true,
            transaction_id: Some("txn1".into()),
        });
        assert!(core.entitlements().pro);
    }

    #[test]
    fn refunding_the_store_purchase_keeps_pro_if_a_license_key_is_still_active() {
        let core = open_core();
        core.activate_license(valid_test_license_key()).unwrap();
        core.set_store_entitlement(StoreEntitlement {
            pro: true,
            transaction_id: Some("txn1".into()),
        });
        // Refund: the store purchase no longer grants Pro.
        core.set_store_entitlement(StoreEntitlement {
            pro: false,
            transaction_id: None,
        });
        assert!(
            core.entitlements().pro,
            "a still-active license key should keep Pro after a refund"
        );
    }

    #[test]
    fn a_pro_store_purchase_keeps_pro_even_without_ever_activating_a_license() {
        let core = open_core();
        core.set_store_entitlement(StoreEntitlement {
            pro: true,
            transaction_id: Some("txn1".into()),
        });
        core.set_store_entitlement(StoreEntitlement {
            pro: true,
            transaction_id: Some("txn2".into()),
        });
        assert!(core.entitlements().pro);
    }

    #[test]
    fn projects_and_rag_lexical_search_end_to_end() {
        let core = open_core();

        // 1. Create project
        let project = core
            .create_project(
                "Research Project".into(),
                Some("Special instructions for research".into()),
            )
            .unwrap();
        assert_eq!(project.name, "Research Project");
        assert_eq!(
            project.instructions,
            Some("Special instructions for research".into())
        );

        // 2. List projects
        let projects = core.list_projects();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, project.id);
        assert_eq!(projects[0].name, "Research Project");

        // 3. Rename project
        core.rename_project(project.id.clone(), "Renamed Research Project".into())
            .unwrap();
        let projects_renamed = core.list_projects();
        assert_eq!(projects_renamed[0].name, "Renamed Research Project");

        // 4. Add document with pages sharing an obvious keyword
        let pages = vec![
            DocumentPage {
                page: 1,
                text: "The swift fox runs across the mountain ridge on a sunny morning.".into(),
            },
            DocumentPage {
                page: 2,
                text: "Quantum computing leverages superposition and quantum entanglement.".into(),
            },
            DocumentPage {
                page: 3,
                text: "PocketAI enables local on-device LLMs and RAG with UniFFI integration."
                    .into(),
            },
        ];
        let doc = core
            .add_document(
                project.id.clone(),
                "notes.txt".into(),
                "text/plain".into(),
                pages,
            )
            .unwrap();
        assert_eq!(doc.name, "notes.txt");
        assert_eq!(doc.project_id, project.id);
        assert_eq!(doc.state, "ready");
        assert!(doc.text_chars > 0);

        // 5. List documents
        let docs = core.list_documents(project.id.clone());
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, doc.id);

        // 6. Search project
        let hits = core.search_project(project.id.clone(), "superposition entanglement".into(), 5);
        assert!(!hits.is_empty(), "expected hits for query terms");
        assert_eq!(hits[0].document_id, doc.id);
        assert!(
            hits[0].text.contains("superposition"),
            "expected text to contain query term: {}",
            hits[0].text
        );
        assert_eq!(hits[0].page, Some(2));
        assert!(hits[0].score > 0.0);

        // Search with k == 0 defaults to limit 20
        let default_limit_hits = core.search_project(project.id.clone(), "quantum".into(), 0);
        assert!(!default_limit_hits.is_empty());

        // Search with non-matching query returns empty
        let empty_hits = core.search_project(project.id.clone(), "nonexistenttermxyz".into(), 5);
        assert!(empty_hits.is_empty());

        // 7. Delete document
        core.delete_document(doc.id.clone()).unwrap();
        assert!(core.list_documents(project.id.clone()).is_empty());
        assert!(core
            .search_project(project.id.clone(), "superposition".into(), 5)
            .is_empty());

        // 8. Delete project
        core.delete_project(project.id.clone()).unwrap();
        assert!(core.list_projects().is_empty());
    }

    #[test]
    fn host_cert_pem_returns_valid_pem_matching_identity() {
        use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
        use base64::Engine;
        use sha2::{Digest, Sha256};

        let core = open_core();
        let identity = core.host_tls_identity();
        let pem = core.host_cert_pem();

        assert!(
            pem.starts_with("-----BEGIN CERTIFICATE-----\n"),
            "expected PEM header, got: {pem}"
        );
        assert!(
            pem.ends_with("-----END CERTIFICATE-----\n"),
            "expected PEM footer, got: {pem}"
        );

        let b64: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let der = STANDARD.decode(&b64).expect("valid base64 DER");

        assert_eq!(der, identity.cert_der);

        let mut hasher = Sha256::new();
        hasher.update(&der);
        let fingerprint = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(fingerprint, identity.fingerprint_b64url);
    }

    /// Wraps `MockEngineBridge` but counts `unload` calls so tests can prove
    /// `PocketCore::unload` routes through to the engine (D35).
    struct UnloadCountingEngine {
        inner: MockEngineBridge,
        unloads: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl PocketKitEngine for UnloadCountingEngine {
        fn kind(&self) -> EngineKind {
            self.inner.kind()
        }
        fn supports(&self, model: ModelManifest) -> bool {
            self.inner.supports(model)
        }
        fn load(
            &self,
            model: ModelManifest,
            opts: LoadOptions,
        ) -> Result<LoadedModel, PocketError> {
            self.inner.load(model, opts)
        }
        fn unload(&self, handle: String) -> Result<(), PocketError> {
            self.unloads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.unload(handle)
        }
        fn generate(
            &self,
            handle: String,
            req: GenerateRequest,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            self.inner.generate(handle, req, sink)
        }
        fn embed(
            &self,
            handle: String,
            inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            self.inner.embed(handle, inputs)
        }
        fn cancel(&self, request_id: String) {
            self.inner.cancel(request_id)
        }
        fn resident_bytes(&self, handle: String) -> u64 {
            self.inner.resident_bytes(handle)
        }
    }

    #[test]
    fn add_document_persists_embeddings_and_search_is_hybrid() {
        use pocket_store::repos::chunks::ChunksRepo;
        use pocket_store::repos::models::{ModelRow, ModelsRepo};

        let core = open_core();
        core.register_engine(Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(
            2000.0,
        ))));

        // An installed embedding model so `default_embedding_model_id` resolves
        // and `embed` routes to the registered engine.
        let embed_id = "mlx-community/bge-small-en-v1.5-4bit@6d66cdf333ff5d4cdad084f2fc2a619575d064ab";
        ModelsRepo::new(&core.store_for_test())
            .upsert(&ModelRow {
                id: embed_id.into(),
                manifest_json: "{}".into(),
                state: "installed".into(),
                bytes_done: 0,
                installed_at: Some(1),
                last_used_at: None,
                pinned_engine: None,
                benchmark_json: None,
            })
            .unwrap();

        let project = core
            .create_project("Docs".into(), None)
            .unwrap();
        let doc = core
            .add_document(
                project.id.clone(),
                "manual".into(),
                "text/plain".into(),
                vec![DocumentPage {
                    page: 1,
                    text: "python installation requires version 3.10 or later".into(),
                }],
            )
            .unwrap();

        // Document records which model embedded it, and chunks carry vectors.
        assert_eq!(doc.embed_model.as_deref(), Some(embed_id));
        let rows = ChunksRepo::new(&core.store_for_test())
            .list_for_document(&doc.id)
            .unwrap();
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|r| !r.embedding.is_empty()),
            "every chunk should have a persisted embedding"
        );

        let hits = core.search_project(project.id, "python installation".into(), 5);
        assert!(!hits.is_empty());
    }

    #[test]
    fn unload_frees_tracked_handles_and_is_idempotent() {
        use std::sync::atomic::Ordering;

        let core = open_core();
        let unloads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        core.register_engine(Arc::new(UnloadCountingEngine {
            inner: MockEngineBridge(pocket_engine::MockEngine::new(2000.0)),
            unloads: unloads.clone(),
        }));

        let model_id =
            "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e".to_string();
        install_model(&core, &model_id);
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Test".into()),
                project_id: None,
                model_id: Some(model_id.clone()),
            })
            .unwrap();
        let sink = Arc::new(CollectingChatSink {
            deltas: Mutex::new(vec![]),
            saved: Mutex::new(None),
        });
        core.send(
            conversation.id,
            UserInput {
                parts: vec![ContentPart::Text { text: "Hi".into() }],
            },
            sink,
        )
        .unwrap();

        // Unloading a model that was never loaded is a no-op success.
        core.unload("never-loaded".into()).unwrap();
        assert_eq!(unloads.load(Ordering::SeqCst), 0);

        // Unloading the loaded model frees it exactly once.
        core.unload(model_id.clone()).unwrap();
        assert_eq!(unloads.load(Ordering::SeqCst), 1);

        // Idempotent: the handles were already drained.
        core.unload(model_id).unwrap();
        assert_eq!(unloads.load(Ordering::SeqCst), 1);
    }

    /// Wraps `MockEngineBridge` but counts `load` calls so tests can prove a
    /// second generation reuses the already-loaded handle instead of loading a
    /// fresh multi-GB copy per message.
    struct LoadCountingEngine {
        inner: MockEngineBridge,
        loads: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl PocketKitEngine for LoadCountingEngine {
        fn kind(&self) -> EngineKind {
            self.inner.kind()
        }
        fn supports(&self, model: ModelManifest) -> bool {
            self.inner.supports(model)
        }
        fn load(
            &self,
            model: ModelManifest,
            opts: LoadOptions,
        ) -> Result<LoadedModel, PocketError> {
            self.loads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.load(model, opts)
        }
        fn unload(&self, handle: String) -> Result<(), PocketError> {
            self.inner.unload(handle)
        }
        fn generate(
            &self,
            handle: String,
            req: GenerateRequest,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            self.inner.generate(handle, req, sink)
        }
        fn embed(
            &self,
            handle: String,
            inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            self.inner.embed(handle, inputs)
        }
        fn cancel(&self, request_id: String) {
            self.inner.cancel(request_id)
        }
        fn resident_bytes(&self, handle: String) -> u64 {
            self.inner.resident_bytes(handle)
        }
    }

    #[test]
    fn second_send_reuses_loaded_handle_and_memory_warning_frees_it() {
        use std::sync::atomic::Ordering;

        let core = open_core();
        let loads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        core.register_engine(Arc::new(LoadCountingEngine {
            inner: MockEngineBridge(pocket_engine::MockEngine::new(2000.0)),
            loads: loads.clone(),
        }));

        let model_id =
            "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e".to_string();
        install_model(&core, &model_id);
        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Reuse".into()),
                project_id: None,
                model_id: Some(model_id),
            })
            .unwrap();

        let send = |text: &str| {
            let sink = Arc::new(CollectingChatSink {
                deltas: Mutex::new(vec![]),
                saved: Mutex::new(None),
            });
            core.send(
                conversation.id.clone(),
                UserInput {
                    parts: vec![ContentPart::Text { text: text.into() }],
                },
                sink,
            )
            .unwrap();
        };

        send("Hello!");
        assert_eq!(loads.load(Ordering::SeqCst), 1, "first send loads once");
        send("And again?");
        assert_eq!(
            loads.load(Ordering::SeqCst),
            1,
            "second send must reuse the loaded handle, not load a fresh copy"
        );

        // A memory warning drains the resident handle; the next send reloads.
        core.on_memory_warning();
        send("After the warning");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    const BENCH_MODEL_ID: &str =
        "mlx-community/Qwen3-1.7B-4bit@3b1b1768f8f8cf8351c712464f906e86c2b8269e";

    #[test]
    fn benchmark_model_measures_persists_and_reads_back() {
        let core = open_core();
        core.register_engine(Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(
            2000.0,
        ))));
        install_model(&core, BENCH_MODEL_ID);

        // Nothing measured yet, so the UI has nothing to show.
        assert!(core.stored_benchmark(BENCH_MODEL_ID.into()).is_none());

        let benchmark = core.benchmark_model(BENCH_MODEL_ID.into()).unwrap();
        assert!(benchmark.tps > 0.0, "a run must report real throughput");

        // Persisted to the model row, so it survives a relaunch.
        let stored = core
            .stored_benchmark(BENCH_MODEL_ID.into())
            .expect("the run must be persisted");
        assert_eq!(stored.tps, benchmark.tps);
        assert_eq!(stored.ttft_ms, benchmark.ttft_ms);

        // And it turns the catalog's fit estimate into a measured one (ENG-09).
        let entry = core
            .catalog(CatalogFilter {
                modality: None,
                curated_only: false,
                installed_only: true,
            })
            .into_iter()
            .find(|e| e.manifest.id == BENCH_MODEL_ID)
            .expect("the installed model must appear in the catalog");
        assert!(matches!(entry.fit.confidence, Confidence::Measured));
        assert_eq!(entry.fit.measured_tokens_per_sec, Some(benchmark.tps));
    }

    #[test]
    fn benchmark_model_rejects_an_unknown_model() {
        let core = open_core();
        core.register_engine(Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(
            2000.0,
        ))));
        assert!(matches!(
            core.benchmark_model("nobody/such-model@deadbeef".into()),
            Err(PocketError::NotFound { .. })
        ));
    }

    /// Calls `benchmark_model` from inside `generate`, i.e. while a reply is
    /// actually in flight, and records what it said. A benchmark run next to
    /// real decoding would measure contention, not the model.
    struct ReentrantBenchmarkEngine {
        inner: MockEngineBridge,
        core: Mutex<std::sync::Weak<PocketCore>>,
        result: Arc<Mutex<Option<Result<Benchmark, PocketError>>>>,
    }

    impl PocketKitEngine for ReentrantBenchmarkEngine {
        fn kind(&self) -> EngineKind {
            self.inner.kind()
        }
        fn supports(&self, model: ModelManifest) -> bool {
            self.inner.supports(model)
        }
        fn load(
            &self,
            model: ModelManifest,
            opts: LoadOptions,
        ) -> Result<LoadedModel, PocketError> {
            self.inner.load(model, opts)
        }
        fn unload(&self, handle: String) -> Result<(), PocketError> {
            self.inner.unload(handle)
        }
        fn generate(
            &self,
            handle: String,
            req: GenerateRequest,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            if let Some(core) = self.core.lock().unwrap().upgrade() {
                let mut slot = self.result.lock().unwrap();
                if slot.is_none() {
                    *slot = Some(core.benchmark_model(BENCH_MODEL_ID.into()));
                }
            }
            self.inner.generate(handle, req, sink)
        }
        fn embed(
            &self,
            handle: String,
            inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, PocketError> {
            self.inner.embed(handle, inputs)
        }
        fn cancel(&self, request_id: String) {
            self.inner.cancel(request_id)
        }
        fn resident_bytes(&self, handle: String) -> u64 {
            self.inner.resident_bytes(handle)
        }
    }

    #[test]
    fn benchmark_model_refuses_mid_generation() {
        let core = open_core();
        let result: Arc<Mutex<Option<Result<Benchmark, PocketError>>>> = Arc::new(Mutex::new(None));
        core.register_engine(Arc::new(ReentrantBenchmarkEngine {
            inner: MockEngineBridge(pocket_engine::MockEngine::new(2000.0)),
            core: Mutex::new(Arc::downgrade(&core)),
            result: Arc::clone(&result),
        }));
        install_model(&core, BENCH_MODEL_ID);

        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Busy".into()),
                project_id: None,
                model_id: Some(BENCH_MODEL_ID.into()),
            })
            .unwrap();
        core.send(
            conversation.id,
            UserInput {
                parts: vec![ContentPart::Text {
                    text: "Hello!".into(),
                }],
            },
            Arc::new(CollectingChatSink {
                deltas: Mutex::new(vec![]),
                saved: Mutex::new(None),
            }),
        )
        .unwrap();

        let attempted = result.lock().unwrap().take().expect("benchmark attempted");
        assert!(matches!(attempted, Err(PocketError::InvalidInput { .. })));
        // And it left no half-measured result behind.
        assert!(core.stored_benchmark(BENCH_MODEL_ID.into()).is_none());
    }

    #[test]
    fn model_supports_reasoning_uses_catalog_then_heuristic() {
        use crate::model_supports_reasoning as r;
        const VL: &str = "mlx-community/Qwen3-VL-4B-Instruct-4bit@2fd8dacbdb8f1e54b8c005f081ec5bf79c56376b";
        // Curated: the catalog flag wins over the name heuristic (Qwen3 → true).
        assert!(!r(VL.into()));
        assert!(!r(format!("mac:peer1/{VL}")));
        assert!(r("mac:peer1/mlx-community/Qwen3-8B-4bit@545dc4251c05440727734bcd94334791f6ab0192".into()));
        // CORE-15: Mac-only ids absent from the catalog keep the heuristic.
        assert!(r("mac:peer1/mlx-community/QwQ-32B-4bit".into()));
        assert!(r("mlx-community/Qwen3-32B-4bit@unknown".into()));
        assert!(!r("mac:peer1/mlx-community/Llama-3.3-70B-Instruct-4bit".into()));
    }

    /// Streams a token every 20 ms until `cancel` is called (or 20 s pass).
    struct UntilCancelledEngine(Arc<std::sync::atomic::AtomicBool>);

    impl PocketKitEngine for UntilCancelledEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::Mlx
        }
        fn supports(&self, _model: ModelManifest) -> bool {
            true
        }
        fn load(&self, _m: ModelManifest, _o: LoadOptions) -> Result<LoadedModel, PocketError> {
            Ok(LoadedModel { handle: "h".into() })
        }
        fn unload(&self, _handle: String) -> Result<(), PocketError> {
            Ok(())
        }
        fn generate(
            &self,
            _handle: String,
            _req: GenerateRequest,
            sink: Arc<dyn PocketKitTokenSink>,
        ) -> Result<GenerateStats, PocketError> {
            use std::sync::atomic::Ordering;
            for _ in 0..1000 {
                if self.0.load(Ordering::SeqCst) {
                    break;
                }
                sink.on_delta(GenerateDelta {
                    text: "tok ".into(),
                    reasoning: String::new(),
                    tool_calls: vec![],
                });
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(PocketError::Cancelled)
        }
        fn embed(&self, _h: String, _i: Vec<String>) -> Result<Vec<Vec<f32>>, PocketError> {
            Ok(vec![])
        }
        fn cancel(&self, _request_id: String) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn resident_bytes(&self, _handle: String) -> u64 {
            0
        }
    }

    /// NET-18: a client that vanishes mid-stream stops the Mac's generation.
    #[test]
    fn dropped_stream_client_cancels_the_generation() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        let core = open_core();
        let cancelled = Arc::new(AtomicBool::new(false));
        core.register_engine(Arc::new(UntilCancelledEngine(Arc::clone(&cancelled))));
        let model = first_catalog_model_id(&core);
        install_model(&core, &model);
        let status = core
            .server_start(ServerConfig {
                lan_enabled: false,
                port: 0,
            })
            .unwrap();
        {
            let (mut conn, mut sock) = open_tls_stream(status.port);
            let mut stream = rustls::Stream::new(&mut conn, &mut sock);
            let body = serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "go"}],
                "stream": true,
            })
            .to_string();
            let request = format!(
                "POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).unwrap();
            let mut buf = [0u8; 512];
            // Only the role chunk: the client may be gone before the engine
            // has even registered the request - that must still cancel it.
            assert!(stream.read(&mut buf).unwrap() > 0);
        } // client gone
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
        while !cancelled.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(cancelled.load(Ordering::SeqCst), "generation kept running");
        core.server_stop().unwrap();
    }

    mod sync_tests {
        //! NET-22: iPhone `sync_now` against a real Mac server on loopback TLS.
        use super::*;
        use crate::traits::SyncTransport;
        use pocket_store::{ConversationRow, ConversationsRepo, MessageRow, MessagesRepo};

        /// The Swift `PeerClient` stand-in: raw HTTP/1.1 over TLS to the Mac.
        struct Loopback(u16);

        impl SyncTransport for Loopback {
            fn request(&self, method: String, path: String, body: Vec<u8>) -> Result<Vec<u8>, PocketError> {
                use std::io::{Read, Write};
                let (mut conn, mut sock) = open_tls_stream(self.0);
                let mut stream = rustls::Stream::new(&mut conn, &mut sock);
                let head = format!(
                    "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
                let mut buf = Vec::new();
                stream.read_to_end(&mut buf).unwrap();
                let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
                let status: u16 = String::from_utf8_lossy(&buf[..split])
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .parse()
                    .unwrap();
                match status {
                    200..=299 => Ok(buf[split + 4..].to_vec()),
                    403 => Err(PocketError::NotEntitled { feature: "chat sync".into() }),
                    _ => Err(PocketError::Network { message: format!("HTTP {status} for {path}") }),
                }
            }
        }

        fn mac(pro: bool) -> (Arc<PocketCore>, Arc<Loopback>) {
            let core = open_core();
            if pro {
                core.activate_license(valid_test_license_key()).unwrap();
            }
            let status = core.server_start(ServerConfig { lan_enabled: false, port: 0 }).unwrap();
            (core, Arc::new(Loopback(status.port)))
        }

        fn add_chat(core: &PocketCore, id: &str, texts: &[&str]) {
            let store = core.store_for_test();
            ConversationsRepo::new(store)
                .create(&ConversationRow {
                    id: id.into(),
                    title: Some(format!("chat {id}")),
                    project_id: None,
                    model_id: None,
                    system_prompt: None,
                    params_json: None,
                    created_at: 1,
                    updated_at: 1,
                    archived: false,
                })
                .unwrap();
            for (i, text) in texts.iter().enumerate() {
                MessagesRepo::new(store).create(&message(&format!("{id}-m{i}"), id, i as i64, text)).unwrap();
            }
        }

        fn message(id: &str, conv: &str, t: i64, text: &str) -> MessageRow {
            MessageRow {
                id: id.into(),
                conversation_id: conv.into(),
                role: "user".into(),
                content_json: serde_json::json!({ "text": text }).to_string(),
                reasoning: None,
                stats_json: None,
                model_id: None,
                created_at: t,
                status: "done".into(),
            }
        }

        fn count(core: &PocketCore, sql: &str) -> i64 {
            core.store_for_test().writer().query_row(sql, [], |r| r.get(0)).unwrap()
        }

        fn all_chats(core: &PocketCore) -> Vec<String> {
            let mut out: Vec<String> = core
                .conversations(ConversationQuery { search: None, archived: None, limit: 100_000, offset: 0 })
                .into_iter()
                .map(|c| format!("{} {:?} {}", c.id, c.title, core.messages(c.id.clone()).len()))
                .collect();
            out.sort();
            out
        }

        #[test]
        fn phone_and_mac_converge_both_ways_with_blobs_then_go_quiet() {
            let (mac, link) = mac(true);
            let phone = open_core();
            add_chat(&phone, "p1", &["from phone"]);
            add_chat(&mac, "m1", &["from mac", "again"]);
            // A photo attached on the phone.
            let img = phone.store_for_test().blob_dir().unwrap().with_file_name("photo.jpg");
            std::fs::write(&img, b"photo bytes").unwrap();
            let mut with_photo = message("p1-img", "p1", 5, "look");
            with_photo.content_json = serde_json::json!({ "text": "look", "parts": [{ "type": "imageRef", "path": img.to_string_lossy() }] }).to_string();
            MessagesRepo::new(phone.store_for_test()).create(&with_photo).unwrap();

            let report = phone.sync_now("mac".into(), link.clone()).unwrap();
            assert!(report.pulled > 0 && report.pushed > 0);
            assert_eq!((report.blobs_fetched, report.blobs_sent), (0, 1));
            assert_eq!(all_chats(&phone), all_chats(&mac));
            assert_eq!(all_chats(&mac).len(), 2);

            let on_mac = mac.messages("p1".into());
            let ContentPart::ImageRef { path } = &on_mac.last().unwrap().parts[0] else { panic!("image part") };
            assert_eq!(std::fs::read(path).unwrap(), b"photo bytes");

            let quiet = phone.sync_now("mac".into(), link).unwrap();
            assert_eq!((quiet.pulled, quiet.pushed, quiet.blobs_sent), (0, 0, 0));
            let status = phone.sync_status("mac".into());
            assert_eq!(status.pending_ops, 0);
            assert!(status.last_synced_at.is_some());
            mac.server_stop().unwrap();
        }

        #[test]
        fn offline_edits_queue_in_the_outbox_and_deletes_propagate() {
            let (mac, link) = mac(true);
            let phone = open_core();
            add_chat(&phone, "c1", &["hi"]);
            add_chat(&phone, "c2", &["keep"]);
            phone.sync_now("mac".into(), link.clone()).unwrap();

            // Offline on both sides.
            phone.delete_conversation("c1".into()).unwrap();
            mac.rename_conversation("c2".into(), "renamed on mac".into()).unwrap();
            let outbox = phone.sync_status("mac".into());
            assert_eq!(outbox.pending_conversation_ids, vec!["c1".to_string()]);

            phone.sync_now("mac".into(), link).unwrap();
            assert!(mac.get_conversation("c1".into()).is_none());
            assert_eq!(phone.get_conversation("c2".into()).unwrap().title.as_deref(), Some("renamed on mac"));
            assert_eq!(phone.sync_status("mac".into()).pending_ops, 0);
            mac.server_stop().unwrap();
        }

        #[test]
        fn forget_and_re_pair_gives_no_duplicates() {
            let (mac, link) = mac(true);
            let phone = open_core();
            add_chat(&phone, "c1", &["a", "b"]);
            add_chat(&mac, "c2", &["c"]);
            phone.sync_now("mac".into(), link.clone()).unwrap();
            let ops = count(&mac, "SELECT COUNT(*) FROM sync_ops");

            // Forget + re-pair: new peer record, same replicas.
            phone.revoke_peer("mac".into()).unwrap();
            let again = phone.sync_now("mac-repaired".into(), link).unwrap();
            assert_eq!((again.pulled, again.pushed), (0, 0));
            assert_eq!(count(&mac, "SELECT COUNT(*) FROM sync_ops"), ops);
            assert_eq!(count(&mac, "SELECT COUNT(*) FROM messages"), 3);
            assert_eq!(all_chats(&phone), all_chats(&mac));
            mac.server_stop().unwrap();
        }

        #[test]
        fn wait_wakes_on_mac_change_and_on_nudge() {
            use crate::sync_routes::WaitResponse;
            let (mac, link) = mac(true);
            let wait = |since: Option<i64>| -> (i64, std::time::Duration) {
                let t = std::time::Instant::now();
                let path = match since {
                    Some(c) => format!("/pocket/v1/sync/wait?since={c}"),
                    None => "/pocket/v1/sync/wait".into(),
                };
                let body = link.request("GET".into(), path, Vec::new()).unwrap();
                (serde_json::from_slice::<WaitResponse>(&body).unwrap().cursor, t.elapsed())
            };
            // No cursor yet: answers at once.
            let (c0, took) = wait(None);
            assert!(took.as_millis() < 1000);

            // A chat written on the Mac wakes a waiting phone well under 1 s.
            let writer = {
                let mac = mac.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    add_chat(&mac, "m1", &["hi"]);
                })
            };
            let (c1, took) = wait(Some(c0));
            writer.join().unwrap();
            assert!(c1 > c0);
            assert!(took.as_millis() < 1300, "{took:?}");

            // Menu-bar "Sync now" is a nudge the waiting phone sees.
            let nudger = {
                let mac = mac.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    mac.sync_nudge();
                })
            };
            let (c2, took) = wait(Some(c1));
            nudger.join().unwrap();
            assert_eq!(c2, c1 + 1);
            assert!(took.as_millis() < 1300, "{took:?}");
            mac.server_stop().unwrap();
        }

        #[test]
        fn per_device_toggle_answers_sync_disabled_not_not_entitled() {
            let (mac, _link) = mac(true);
            assert!(mac.sync_enabled_for_device("phone-1".into()));
            mac.set_sync_enabled_for_device("phone-1".into(), false).unwrap();
            assert!(!mac.sync_enabled_for_device("phone-1".into()));
            assert!(mac.sync_enabled_for_device("phone-2".into()));
            let e = mac.sync_store(Some("phone-1")).err().unwrap();
            assert!(matches!(e, PocketError::SyncDisabled));
            let (status, body) = crate::control_routes::error_response(e);
            assert_eq!(status.as_u16(), 403);
            assert_eq!(body.0["error"]["code"], "sync_disabled");
            assert!(mac.sync_store(Some("phone-2")).is_ok());
            mac.set_sync_enabled_for_device("phone-1".into(), true).unwrap();
            assert!(mac.sync_store(Some("phone-1")).is_ok());
            mac.server_stop().unwrap();
        }

        #[test]
        fn mac_reports_applied_ops_and_forget_can_remove_the_phones_chats() {
            struct Events(Mutex<Vec<String>>);
            impl crate::traits::EventSink for Events {
                fn on_event(&self, event: Event) {
                    self.0.lock().unwrap().push(format!("{} {}", event.event_type, event.data_json));
                }
            }
            let (mac, link) = mac(true);
            let events = Arc::new(Events(Mutex::new(Vec::new())));
            mac.subscribe(events.clone());
            let phone = open_core();
            add_chat(&phone, "p1", &["from phone"]);
            add_chat(&mac, "m1", &["from mac"]);
            phone.sync_now("mac".into(), link).unwrap();
            let seen = events.0.lock().unwrap().clone();
            assert!(seen.iter().any(|e| e.starts_with("sync.applied") && e.contains("\"applied\":2")), "{seen:?}");

            // Each side removes only what the other created, locally.
            let phone_sync_id = pocket_store::sync::device_id(phone.store_for_test()).unwrap();
            assert_eq!(mac.remove_synced_chats(phone_sync_id).unwrap(), 1);
            assert_eq!(phone.remove_synced_chats("mac".into()).unwrap(), 1);
            assert!(mac.get_conversation("p1".into()).is_none());
            assert!(mac.get_conversation("m1".into()).is_some());
            assert!(phone.get_conversation("m1".into()).is_none());
            assert!(phone.get_conversation("p1".into()).is_some());
            assert_eq!(phone.remove_synced_chats("never-synced".into()).unwrap(), 0);
            mac.server_stop().unwrap();
        }

        #[test]
        fn free_mac_rejects_sync() {
            let (mac, link) = mac(false);
            let phone = open_core();
            add_chat(&phone, "c1", &["hi"]);
            let err = phone.sync_now("mac".into(), link).unwrap_err();
            assert!(matches!(err, PocketError::NotEntitled { .. }), "{err:?}");
            assert!(mac.get_conversation("c1".into()).is_none());
            // Nothing was pushed, so it is all still in the phone's outbox.
            assert_eq!(phone.sync_status("mac".into()).pending_conversation_ids, vec!["c1".to_string()]);
            mac.server_stop().unwrap();
        }

        #[test]
        fn a_thousand_legacy_chats_seed_and_sync_in_under_30s() {
            let (mac, link) = mac(true);
            let phone = open_core();
            {
                // Pre-NET-22 rows: no ops yet, so the first sync seeds them.
                let mut conn = phone.store_for_test().writer();
                let tx = conn.transaction().unwrap();
                for i in 0..1000 {
                    tx.execute(
                        "INSERT INTO conversations (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                        rusqlite::params![format!("c{i}"), format!("chat {i}"), i],
                    )
                    .unwrap();
                    for j in 0..4 {
                        tx.execute(
                            "INSERT INTO messages (id, conversation_id, role, content_json, created_at, status)
                             VALUES (?1, ?2, ?3, ?4, ?5, 'done')",
                            rusqlite::params![format!("c{i}-m{j}"), format!("c{i}"), if j % 2 == 0 { "user" } else { "assistant" },
                                serde_json::json!({ "text": "x".repeat(400) }).to_string(), j],
                        )
                        .unwrap();
                    }
                }
                tx.commit().unwrap();
            }
            let started = std::time::Instant::now();
            let report = phone.sync_now("mac".into(), link.clone()).unwrap();
            let took = started.elapsed();
            assert_eq!(report.pushed, 5000);
            assert!(took < std::time::Duration::from_secs(30), "took {took:?}");
            assert_eq!(count(&mac, "SELECT COUNT(*) FROM conversations"), 1000);
            assert_eq!(count(&mac, "SELECT COUNT(*) FROM messages"), 4000);
            let again = phone.sync_now("mac".into(), link).unwrap();
            assert_eq!((again.pulled, again.pushed), (0, 0));
            assert_eq!(count(&mac, "SELECT COUNT(*) FROM messages"), 4000);
            eprintln!("1,000 chats / 5,000 ops seeded + synced in {took:?}");
            mac.server_stop().unwrap();
        }
    }
}
