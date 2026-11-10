//! Scout planning, scout lifecycle, and service implementation.
//!
//! The files are grouped by responsibility. They are included in this shared
//! implementation module so the existing tightly-coupled private lifecycle
//! helpers remain private and the crate API stays unchanged.

include!("contracts.rs");
include!("candidate_planning.rs");
include!("service.rs");
include!("scout_handlers.rs");
include!("scout_report.rs");

#[cfg(test)]
mod tests {
    use super::*;

    async fn check_outbox_acknowledgement(successes: usize) {
        use std::sync::atomic::{AtomicUsize, Ordering};
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
        let requests = Arc::new(AtomicUsize::new(0));
        let stored_at_failure = Arc::new(AtomicUsize::new(usize::MAX));
        let counter = requests.clone();
        let observed = stored_at_failure.clone();
        let db_path = path.clone();
        let app = axum::Router::new().route(
            "/feedback",
            axum::routing::post(move || {
                let counter = counter.clone();
                let observed = observed.clone();
                let db_path = db_path.clone();
                async move {
                    if counter.fetch_add(1, Ordering::SeqCst) < successes {
                        return StatusCode::OK;
                    }
                    let db = rusqlite::Connection::open(db_path).unwrap();
                    let count: i64 = db
                        .query_row(
                            "SELECT COUNT(*) FROM scout_records WHERE collection='outbox'",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap();
                    observed.store(count as usize, Ordering::SeqCst);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        assert!(
            flush_scout_outbox(
                &scout,
                &reqwest::Client::new(),
                &format!("http://{address}")
            )
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
        assert!(loaded.seen.contains("event-1"));
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
            client: reqwest::Client::new(),
            learning_url: String::new(),
            auto_trader_url: String::new(),
            seen: Arc::new(Mutex::new(BTreeSet::new())),
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
