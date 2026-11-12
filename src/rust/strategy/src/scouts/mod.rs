//! Scout planning, scout lifecycle, and service implementation.
//!
//! The files are grouped by responsibility. They are included in this shared
//! implementation module so the existing tightly-coupled private lifecycle
//! helpers remain private and the crate API stays unchanged.

#[cfg(feature = "service")]
use crate::parameter_scouting::{
    ParameterVariantsRequest, StrategyEvidenceCandidate, candidate_value, combination_context,
    combination_key, context_hash, next_parameter_combination, next_parameter_probe,
    parse_combination, probe_id, scoutable_specs, single_parameter_variants,
};

include!("contracts.rs");
include!("candidate_planning.rs");
include!("service.rs");
include!("scout_handlers.rs");
include!("scout_report.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scout_instance_cache_stays_bounded_without_dropping_database_rows() {
        let path = std::env::temp_dir().join(format!(
            "scout-instance-cache-bound-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        for index in 0..(MAX_CACHED_SCOUT_INSTANCES + 17) {
            let decision_id = format!("decision-{index:05}");
            persisted.scout_instances.insert(
                decision_id.clone(),
                serde_json::json!({
                    "decision_id":decision_id,
                    "strategy":"live-momentum",
                    "symbol":"BTC-USDT",
                    "timestamp":index as u64 + 1,
                    "direction":"bullish",
                    "scout_entry_price":100.0
                }),
            );
        }
        persist_scout_state(&path, &persisted, &["scout_instances"]).unwrap();

        let mut runtime = load_scout_state(&path).unwrap();
        assert!(runtime.scout_instances.is_empty());
        let report_rows = load_scout_instances_since(&path, 0).unwrap();
        assert_eq!(report_rows.len(), MAX_CACHED_SCOUT_INSTANCES + 17);
        for (decision_id, instance) in report_rows {
            cache_scout_instance(&mut runtime, &decision_id, instance);
        }
        assert_eq!(runtime.scout_instances.len(), MAX_CACHED_SCOUT_INSTANCES);
        assert_eq!(
            rusqlite::Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM scout_records WHERE collection='scout_instances'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap() as usize,
            MAX_CACHED_SCOUT_INSTANCES + 17
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn scout_worker_startup_does_not_rewrite_persisted_instance_history() {
        let path = std::env::temp_dir().join(format!(
            "scout-startup-preserves-payload-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        persisted.scout_instances.insert(
            "historical-decision".into(),
            serde_json::json!({
                "decision_id":"historical-decision",
                "strategy":"live-momentum",
                "symbol":"BTC-USDT",
                "timestamp":1,
                "legacy_evidence_payload":{"must":"remain byte-for-byte durable"}
            }),
        );
        persist_scout_state(&path, &persisted, &["scout_instances"]).unwrap();
        let read_payload = || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT payload FROM scout_records WHERE collection='scout_instances' AND record_key='historical-decision'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        };
        let original = read_payload();

        let strategy = strategies::Strategy::from_name("live-momentum").unwrap();
        let _router = router_with_state(path.clone(), strategy);

        assert_eq!(read_payload(), original);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn plan_result_cache_is_bounded_and_old_idempotent_results_remain_in_sqlite() {
        let path = std::env::temp_dir().join(format!(
            "scout-plan-result-cache-bound-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        for index in 0..(MAX_CACHED_SCOUT_PLAN_RESULTS + 9) {
            persisted.scout_plan_results.insert(
                format!("request-{index:03}"),
                ScoutPlanResponse {
                    audits: vec![serde_json::json!({"decision_id": format!("decision-{index}")})],
                    selected_groups: vec![format!("group-{index}")],
                },
            );
        }
        persist_scout_state(&path, &persisted, &["scout_plan_results"]).unwrap();

        let hydrated = load_scout_state(&path).unwrap();
        assert_eq!(
            hydrated.scout_plan_results.len(),
            MAX_CACHED_SCOUT_PLAN_RESULTS
        );
        assert!(!hydrated.scout_plan_results.contains_key("request-000"));
        let old_retry: ScoutPlanResponse =
            load_scout_record(&path, "scout_plan_results", "request-000")
                .unwrap()
                .expect("old retry result remains available from SQLite");
        assert_eq!(old_retry.selected_groups, vec!["group-0"]);
        assert_eq!(
            rusqlite::Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM scout_records WHERE collection='scout_plan_results'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap() as usize,
            MAX_CACHED_SCOUT_PLAN_RESULTS + 9
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn active_audit_hydration_is_bounded_while_registration_replay_pages_all_rows() {
        let path = std::env::temp_dir().join(format!(
            "scout-active-audit-cache-bound-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let now = scout_now_ms();
        let mut persisted = PersistedState::default();
        for index in 0..(MAX_ACTIVE_SCOUT_AUDIT_GROUPS + 9) {
            let group = format!("group-{index:03}");
            persisted.scout_active_audits.insert(
                group.clone(),
                vec![serde_json::json!({
                    "decision_id": format!("decision-{index}"),
                    "symbol":"BTC-USDT",
                    "strategy":"trend-50-200",
                    "direction":"bullish",
                    "timestamp":now,
                    "scout_entry_price":100.0,
                    "scout_sample_group_id":group,
                    "strategy_decision":{"strategy_parameters":{}}
                })],
            );
        }
        persist_scout_state(&path, &persisted, &["scout_active_audits"]).unwrap();

        let hydrated = load_scout_state(&path).unwrap();
        assert_eq!(
            hydrated.scout_active_audits.len(),
            MAX_ACTIVE_SCOUT_AUDIT_GROUPS
        );
        let mut cursor = String::new();
        let mut replayed = 0;
        loop {
            let (registrations, next) =
                load_active_scout_registration_page(&path, &cursor).unwrap();
            replayed += registrations.len();
            let Some(next) = next else { break };
            cursor = next;
        }
        assert_eq!(replayed, MAX_ACTIVE_SCOUT_AUDIT_GROUPS + 9);
        assert_eq!(
            rusqlite::Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM scout_records WHERE collection='scout_active_audits'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap() as usize,
            MAX_ACTIVE_SCOUT_AUDIT_GROUPS + 9
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn strategy_state_import_partitions_records_and_never_overwrites_existing_destination() {
        let directory =
            std::env::temp_dir().join(format!("scout-partition-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("legacy.json");
        let destination = directory.join("live-momentum.sqlite");
        std::fs::write(&source, serde_json::json!({
            "scout_configuration":{"strategies":["live-momentum","live-trade-flow"]},
            "seen":["live-scout:live-momentum:BTC", "live-scout:live-trade-flow:ETH"],
            "outcomes":{},
            "scout_instances":{
                "m":{"strategy":"live-momentum","symbol":"BTC-USDT"},
                "t":{"strategy":"live-trade-flow","symbol":"ETH-USDT"}
            }, "pairs":{}, "outbox":[],
            "scout_sample_at_ms":{"live-scout:live-momentum|BTC-USDT|bullish":10,"live-scout:live-trade-flow|ETH-USDT|bearish":20},
            "scout_sample_active_scenarios":{}, "scout_variant_rotation_cursor":1,
            "scout_lifecycle_updated_at_ms":10,"scout_plan_profiles":{},"scout_plan_results":{},
            "scout_pending_samples":{},"scout_observation_results":{},"scout_active_audits":{}
        }).to_string()).unwrap();
        let count =
            import_strategy_partition(&source, &destination, strategies::Strategy::LiveMomentum)
                .unwrap();
        assert_eq!(count, 1);
        let imported = load_scout_state(&destination).unwrap();
        assert_eq!(
            load_scout_instance(&destination, "m").unwrap().unwrap()["strategy"],
            "live-momentum"
        );
        assert!(imported.scout_instances.is_empty());
        assert_eq!(imported.seen.len(), 0);
        assert!(scout_record_exists(&destination, "seen", "live-scout:live-momentum:BTC").unwrap());
        assert_eq!(imported.scout_sample_at_ms.len(), 1);
        assert!(
            import_strategy_partition(&source, &destination, strategies::Strategy::LiveMomentum)
                .is_err()
        );
        let bulk_directory = directory.join("bulk");
        let partitions = import_strategy_partitions(&source, &bulk_directory).unwrap();
        assert_eq!(partitions.len(), 2);
        assert!(
            partitions
                .iter()
                .any(|(strategy, count)| strategy == "live-momentum" && *count == 1)
        );
        assert!(
            partitions
                .iter()
                .any(|(strategy, count)| strategy == "live-trade-flow" && *count == 1)
        );
        let trade_flow = load_scout_state(&bulk_directory.join("live-trade-flow.sqlite")).unwrap();
        assert!(trade_flow.scout_instances.is_empty());
        assert!(
            load_scout_instance(&bulk_directory.join("live-trade-flow.sqlite"), "t")
                .unwrap()
                .is_some()
        );
        assert!(
            load_scout_instance(&bulk_directory.join("live-trade-flow.sqlite"), "m")
                .unwrap()
                .is_none()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    async fn check_outbox_acknowledgement(successes: usize) {
        use anyhow::Context;
        use rsocket_rust::prelude::*;
        use rsocket_rust_transport_websocket::WebsocketServerTransport;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use strategies::{
            ScoutCoordinatorRequest, ScoutCoordinatorResponse, ScoutFeedbackAcknowledgement,
        };

        struct FeedbackResponder {
            successes: usize,
            requests: Arc<AtomicUsize>,
            db_path: std::path::PathBuf,
            persisted_before_failure: Arc<AtomicUsize>,
        }

        #[rsocket_rust::async_trait]
        impl RSocket for FeedbackResponder {
            async fn metadata_push(&self, _request: Payload) -> anyhow::Result<()> {
                anyhow::bail!("unsupported")
            }
            async fn fire_and_forget(&self, _request: Payload) -> anyhow::Result<()> {
                anyhow::bail!("unsupported")
            }
            async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
                let ScoutCoordinatorRequest::Feedback(event) =
                    serde_json::from_slice(request.data().context("missing feedback request")?)?
                else {
                    anyhow::bail!("expected Learning feedback event")
                };
                let event_id = match &event {
                    FeedbackEvent::ScoutOutcome { event_id, .. }
                    | FeedbackEvent::TradeOutcome { event_id, .. }
                    | FeedbackEvent::MissedOpportunityCandidate { event_id, .. } => {
                        event_id.clone()
                    }
                };
                let fail = self.requests.fetch_add(1, Ordering::SeqCst) >= self.successes;
                let acknowledgement = if fail {
                    let db = rusqlite::Connection::open(&self.db_path)?;
                    let count: i64 = db.query_row(
                        "SELECT COUNT(*) FROM scout_records WHERE collection='outbox'",
                        [],
                        |row| row.get(0),
                    )?;
                    self.persisted_before_failure
                        .store(count as usize, Ordering::SeqCst);
                    ScoutFeedbackAcknowledgement {
                        event_id,
                        accepted: false,
                        duplicate: false,
                        status: 500,
                        detail: Some("temporary test failure".into()),
                    }
                } else {
                    ScoutFeedbackAcknowledgement {
                        event_id,
                        accepted: true,
                        duplicate: false,
                        status: 200,
                        detail: None,
                    }
                };
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&ScoutCoordinatorResponse::Feedback(
                            acknowledgement,
                        ))?)
                        .build(),
                ))
            }
            fn request_stream(&self, _request: Payload) -> Flux<anyhow::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }
            fn request_channel(
                &self,
                requests: Flux<anyhow::Result<Payload>>,
            ) -> Flux<anyhow::Result<Payload>> {
                requests
            }
        }

        let path = std::env::temp_dir().join(format!(
            "scouts-outbox-{}-{}-{successes}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        for index in 0..=successes {
            persisted
                .outbox
                .push(ScoutOutboxItem::Learning(FeedbackEvent::ScoutOutcome {
                    event_id: format!("event-{index}"),
                    observed_at_ms: scout_now_ms(),
                    learning_context: None,
                    symbol: "BTC-USDT".into(),
                    strategy: "live-momentum".into(),
                    profile_id: "profile".into(),
                    parameters: BTreeMap::new(),
                    baseline_parameters: BTreeMap::new(),
                    comparison: strategies::ScoutComparisonMetadata::Independent,
                    specs: Vec::new(),
                    net_return_percent: 0.1,
                }));
        }
        persist_scout_state(&path, &persisted, &["outbox"]).unwrap();
        let scout = ScoutState {
            data: Arc::new(Mutex::new(persisted)),
            path: path.clone(),
            flush_lock: Arc::new(Mutex::new(())),
            flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let requests = Arc::new(AtomicUsize::new(0));
        let stored_at_failure = Arc::new(AtomicUsize::new(usize::MAX));
        let responder = FeedbackResponder {
            successes,
            requests: requests.clone(),
            db_path: path.clone(),
            persisted_before_failure: stored_at_failure.clone(),
        };
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(FeedbackResponder {
                    successes: responder.successes,
                    requests: responder.requests.clone(),
                    db_path: responder.db_path.clone(),
                    persisted_before_failure: responder.persisted_before_failure.clone(),
                }))
            }))
            .serve();
        let server = tokio::spawn(async move { server.await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            flush_scout_outbox(&scout, &format!("ws://{address}"), "ws://127.0.0.1:10005",)
                .await
                .is_err()
        );
        assert_eq!(scout.data.lock().await.outbox.len(), 1);
        assert_eq!(load_scout_state(&path).unwrap().outbox.len(), 1);
        if successes >= SCOUT_OUTBOX_ACK_BATCH_SIZE {
            assert_eq!(
                stored_at_failure.load(Ordering::SeqCst),
                1,
                "a full batch must be persisted before the queue drains"
            );
        }
        server.abort();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn outbox_acknowledges_completed_items_on_server_failure() {
        check_outbox_acknowledgement(2).await;
    }

    #[tokio::test]
    async fn outbox_commits_delivered_batches_before_queue_drains() {
        check_outbox_acknowledgement(SCOUT_OUTBOX_ACK_BATCH_SIZE).await;
    }

    #[tokio::test]
    async fn failed_outbox_persistence_keeps_delivered_items_for_retry() {
        let mut persisted = PersistedState::default();
        persisted
            .outbox
            .push(ScoutOutboxItem::Audit(serde_json::json!({"id": "first"})));
        let scout = ScoutState {
            data: Arc::new(Mutex::new(persisted)),
            path: std::path::PathBuf::from("/dev/null/state.sqlite"),
            flush_lock: Arc::new(Mutex::new(())),
            flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        assert!(acknowledge_scout_outbox(&scout, 1).await.is_err());
        assert_eq!(scout.data.lock().await.outbox.len(), 1);
    }

    include!("tests/scout_outcome.rs");
    include!("tests/scout_audit_plan.rs");
    include!("tests/scout_plan.rs");
    include!("tests/dated_carry_lifecycle.rs");
    include!("tests/live_candidates.rs");

    #[test]
    fn probe_ids_and_context_hashes_are_stable_and_exclusions_remove_context() {
        let first = probe_id("BTC-USDT|carry");
        assert_eq!(first, probe_id("BTC-USDT|carry"));
        assert_eq!(first.len(), 16);
        assert_ne!(first, probe_id("ETH-USDT|carry"));

        let profile = BTreeMap::from([
            ("entry_threshold".to_owned(), 1.25),
            ("exit_threshold".to_owned(), 2.5),
        ]);
        let excluded = BTreeMap::from([("entry_threshold".to_owned(), 9.0)]);
        assert_eq!(
            context_hash(&profile, &excluded),
            context_hash(
                &BTreeMap::from([("exit_threshold".to_owned(), 2.5)]),
                &BTreeMap::new()
            )
        );
    }

    #[test]
    fn combination_keys_round_trip_and_reject_malformed_or_out_of_bounds_values() {
        let specs = vec![
            ParameterSpec {
                key: "alpha".into(),
                label: "Alpha".into(),
                default: 1.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "beta".into(),
                label: "Beta".into(),
                default: 2.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
        ];
        let values = BTreeMap::from([("alpha".into(), 1.5), ("beta".into(), 2.5)]);
        let full = BTreeMap::from([
            ("alpha".into(), 1.0),
            ("beta".into(), 2.0),
            ("gamma".into(), 3.0),
        ]);
        let key = combination_key(&values, &full).unwrap();
        assert_eq!(combination_context(&key).unwrap().len(), 16);
        assert_eq!(parse_combination(&key, &specs), Some(values));
        assert!(
            parse_combination("combo:alpha=99,beta=2.5|ctx:0000000000000000", &specs).is_none()
        );
        assert!(combination_context("combo:alpha=1,beta=2|ctx:not-a-hash").is_none());
    }

    #[test]
    fn bounded_parameter_sweeps_cover_single_and_pair_candidates() {
        let specs = vec![
            ParameterSpec {
                key: "alpha".into(),
                label: "Alpha".into(),
                default: 1.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "beta".into(),
                label: "Beta".into(),
                default: 2.0,
                min: 0.0,
                max: 4.0,
                step: 0.5,
                unit: "".into(),
                learning_direction: 1.0,
            },
        ];
        let profile = BTreeMap::new();
        assert!(next_parameter_probe("BTC", "s", &profile, &specs, 0).is_some());
        assert_eq!(
            next_parameter_combination("BTC", "s", &profile, &specs, 0)
                .unwrap()
                .len(),
            2
        );
        let variants = all_parameter_variants(&profile, &specs, 0).unwrap();
        assert_eq!(variants.len(), 3);
        assert!(variants.iter().all(|variant| !variant.is_empty()));
        let too_many = (0..17)
            .map(|index| ParameterSpec {
                key: format!("p{index}"),
                label: format!("P{index}"),
                default: 1.0,
                min: 0.0,
                max: 2.0,
                step: 0.1,
                unit: "".into(),
                learning_direction: 1.0,
            })
            .collect::<Vec<_>>();
        assert!(all_parameter_variants(&profile, &too_many, 0).is_none());
    }

    #[test]
    fn scout_persistence_round_trips_state_and_outbox_dedupes_learning_events() {
        let path = std::env::temp_dir().join(format!(
            "scouts-persistence-test-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "event-1".into(),
            observed_at_ms: 10,
            learning_context: Some("context".into()),
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            baseline_parameters: BTreeMap::new(),
            comparison: strategies::ScoutComparisonMetadata::Independent,
            specs: Vec::new(),
            net_return_percent: 0.5,
        };
        let mut second_event = event.clone();
        if let FeedbackEvent::ScoutOutcome { event_id, .. } = &mut second_event {
            *event_id = "event-2".into();
        }
        let mut state = PersistedState {
            seen: BTreeSet::from(["event-1".into()]),
            outbox: vec![
                ScoutOutboxItem::Learning(event.clone()),
                ScoutOutboxItem::Learning(second_event.clone()),
            ],
            ..PersistedState::default()
        };
        assert!(persist_scout_state(&path, &state, &["seen", "outbox"]).is_ok());
        let loaded = load_scout_state(&path).unwrap();
        let mut value_loaded: PersistedState =
            serde_json::from_value(load_scout_state_value(&path).unwrap()).unwrap();
        value_loaded.seen.clear();
        value_loaded.scout_instances.clear();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&value_loaded).unwrap(),
            "streamed runtime loading must preserve every persisted collection"
        );
        assert!(loaded.seen.is_empty());
        assert!(scout_record_exists(&path, "seen", "event-1").unwrap());
        assert_eq!(loaded.outbox.len(), 2);
        assert_eq!(feedback_event_id(&event), "event-1");
        assert!(outbox_has_learning(&loaded, &event));

        let mut other = event.clone();
        if let FeedbackEvent::ScoutOutcome { event_id, .. } = &mut other {
            *event_id = "event-3".into();
        }
        assert!(!outbox_has_learning(&loaded, &other));
        state.outbox.remove(0);
        assert!(persist_scout_state(&path, &state, &["outbox"]).is_ok());
        let loaded = load_scout_state(&path).unwrap();
        assert_eq!(loaded.outbox.len(), 1);
        assert!(outbox_has_learning(&loaded, &second_event));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scout_persistence_updates_one_plan_result_without_dropping_siblings() {
        let path = std::env::temp_dir().join(format!(
            "scouts-incremental-persistence-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut state = PersistedState::default();
        state.scout_plan_results.insert(
            "request-a".into(),
            ScoutPlanResponse {
                audits: Vec::new(),
                selected_groups: vec!["group-a".into()],
            },
        );
        assert!(persist_scout_state(&path, &state, &["scout_plan_results@request-a"]).is_ok());
        state.scout_plan_results.insert(
            "request-b".into(),
            ScoutPlanResponse {
                audits: Vec::new(),
                selected_groups: vec!["group-b".into()],
            },
        );
        assert!(persist_scout_state(&path, &state, &["scout_plan_results@request-b"]).is_ok());
        let loaded = load_scout_state(&path).unwrap();
        assert_eq!(
            loaded.scout_plan_results["request-a"].selected_groups,
            ["group-a"]
        );
        assert_eq!(
            loaded.scout_plan_results["request-b"].selected_groups,
            ["group-b"]
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn active_audit_replay_builds_registrations_from_loaded_state() {
        let path = std::env::temp_dir().join(format!(
            "scouts-audit-replay-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let audit = serde_json::json!({
            "decision_id": "durable-decision",
            "strategy": "live-momentum",
            "symbol": "BTC-USDT",
            "direction": "bullish",
            "timestamp": scout_now_ms(),
            "scout_entry_price": 100.0,
            "strategy_decision": {"strategy_parameters": {}}
        });
        let mut state = PersistedState::default();
        state
            .scout_active_audits
            .insert("durable-group".into(), vec![audit]);
        assert!(persist_scout_state(&path, &state, &["scout_active_audits@durable-group"]).is_ok());

        let loaded = load_scout_state(&path).unwrap();
        let registrations = active_scout_registrations(&loaded);
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0]["decision_id"], "durable-decision");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn repeated_outbox_flush_triggers_coalesce_while_flush_is_locked() {
        let persisted = PersistedState::default();
        let service = ServiceState {
            learning_rsocket_url: String::new(),
            insights_rsocket_url: String::new(),
            auto_trader_rsocket_url: String::new(),
            strategy_outcomes_by_profile: Arc::clone(&persisted.strategy_outcomes_by_profile),
            portfolio_instances_by_group: Arc::clone(&persisted.portfolio_instances_by_group),
            strategy_outcome_profile_index_ready: Arc::new(std::sync::atomic::AtomicBool::new(
                false,
            )),
            live_scout_maximum_duration_ms: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout: ScoutState {
                data: Arc::new(Mutex::new(persisted)),
                path: std::path::PathBuf::new(),
                flush_lock: Arc::new(Mutex::new(())),
                flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        };
        let flush_guard = service.scout.flush_lock.lock().await;
        for _ in 0..50_000 {
            schedule_scout_outbox_flush(&service);
        }
        assert!(
            service
                .scout
                .flush_scheduled
                .load(std::sync::atomic::Ordering::Acquire)
        );
        drop(flush_guard);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while service
                .scout
                .flush_scheduled
                .load(std::sync::atomic::Ordering::Acquire)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("coalesced Scout flush should complete after the lock is released");
    }
}
