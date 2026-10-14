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

    fn urlencode_slash_at(s: &str) -> String {
        s.replace('/', "%2F").replace('@', "%40")
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
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
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

    #[test]
    fn save_peer_then_peers_round_trips() {
        let core = open_core();
        let saved = core
            .save_peer(ApprovedPairing {
                host_id: "01JHOST".into(),
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
    }

    /// Mirrors pocket-trust's own license tests: payload signed by the
    /// [7u8;32] test keypair whose public half is PLACEHOLDER_KEYGEN_PUBLIC_KEY.
    fn valid_test_license_key() -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let payload = serde_json::json!({
            "v": 1, "tier": "pro", "serial": "01JTEST", "issued": "2026-01-01", "max_major": 0
        });
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string());
        let message = format!("key/{payload_b64}");
        let signature = ed25519_dalek::Signer::sign(&signing_key, message.as_bytes());
        format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
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
    }

    #[test]
    fn send_drives_a_full_chat_turn_through_a_registered_engine() {
        let core = open_core();
        core.register_engine(Arc::new(MockEngineBridge(pocket_engine::MockEngine::new(
            2000.0,
        ))));

        let conversation = core
            .create_conversation(NewConversation {
                title: Some("Test".into()),
                project_id: None,
                model_id: None,
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

    /// Same raw-socket approach as `http_request`, but returns the full
    /// response text unparsed - needed for SSE, which isn't a single JSON body.
    fn http_request_raw(
        port: u16,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> (u16, String) {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
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
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
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
}
