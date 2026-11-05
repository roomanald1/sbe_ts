use std::{
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

use axum::body::Body;
use http_body_util::BodyExt;
use learning::router;
use rusqlite::Connection;
use serde_json::{Value, json};
use tower::ServiceExt;

const VARIANTS: usize = 1_000;
const ACTIVE_PROFILES: usize = 200;
const SEEN_EVENTS: usize = 5_000;

fn profile_json(index: usize) -> Value {
    json!({
        "profile_id": format!("profile-{index}"),
        "version": 1,
        "symbol": format!("ASSET{}-USDT", index % ACTIVE_PROFILES),
        "strategy": format!("strategy-{}", index % 10),
        "parameters": {"entry_threshold": 0.5 + (index % 20) as f64 * 0.01},
        "learning_samples": index % 100,
        "positive_samples": index % 50,
        "realized_pnl": index as f64 * 0.25
    })
}

fn fixture_database(path: &Path) {
    let connection = Connection::open(path).expect("open benchmark database");
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE learning_metadata (id INTEGER PRIMARY KEY CHECK (id = 1), accepted_events INTEGER NOT NULL);
             CREATE TABLE learning_variants (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE learning_active_profiles (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE learning_context_stats (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE learning_execution_stats (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE learning_threshold_samples (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE learning_seen_event_ids (id TEXT PRIMARY KEY);
             CREATE TABLE learning_seen_execution_event_ids (id TEXT PRIMARY KEY);",
        )
        .expect("create benchmark schema");
    connection
        .execute(
            "INSERT INTO learning_metadata VALUES (1, ?1)",
            [SEEN_EVENTS as i64],
        )
        .expect("seed metadata");

    for index in 0..VARIANTS {
        let profile = profile_json(index);
        let registration = json!({
            "profile": profile,
            "specs": [{"key":"entry_threshold", "label":"Entry threshold", "default":0.5, "min":0.1, "max":1.0, "step":0.01, "unit":"score", "learning_direction":1.0}],
            "samples": index % 100,
            "positive": index % 50,
            "realized_pnl": index as f64 * 0.25,
            "outcomes": []
        });
        connection
            .execute(
                "INSERT INTO learning_variants VALUES (?1, ?2)",
                rusqlite::params![format!("variant-{index}"), registration.to_string()],
            )
            .expect("seed variants");
        if index < ACTIVE_PROFILES {
            connection
                .execute(
                    "INSERT INTO learning_active_profiles VALUES (?1, ?2)",
                    rusqlite::params![
                        format!("ASSET{index}-USDT:strategy-{}", index % 10),
                        profile.to_string()
                    ],
                )
                .expect("seed active profiles");
        }
    }
    for index in 0..SEEN_EVENTS {
        connection
            .execute(
                "INSERT INTO learning_seen_event_ids VALUES (?1)",
                [format!("event-{index}")],
            )
            .expect("seed event IDs");
    }
}

fn temp_database(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "learning-bench-{label}-{}.sqlite",
        std::process::id()
    ))
}

fn report(name: &str, started: Instant, iterations: usize) {
    println!(
        "{name}: {:.2} ms/iteration",
        started.elapsed().as_secs_f64() * 1_000.0 / iterations as f64
    );
}

fn load_benchmark() {
    const ITERATIONS: usize = 30;
    let path = temp_database("load");
    let _ = std::fs::remove_file(&path);
    fixture_database(&path);
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = black_box(router(black_box(path.clone())).expect("load learning state"));
    }
    report(
        "learning database load (1,000 variants, 200 profiles, 5,000 event IDs)",
        started,
        ITERATIONS,
    );
    remove_database(&path);
}

fn snapshot_benchmark() {
    const ITERATIONS: usize = 100;
    let path = temp_database("snapshot");
    let _ = std::fs::remove_file(&path);
    fixture_database(&path);
    let app = router(path.clone()).expect("load learning state");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("create benchmark runtime");
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let request = axum::http::Request::builder()
            .uri("/decision-snapshot")
            .body(Body::empty())
            .expect("build snapshot request");
        let response = runtime
            .block_on(app.clone().oneshot(request))
            .expect("snapshot request");
        let body = runtime
            .block_on(response.into_body().collect())
            .expect("read snapshot body")
            .to_bytes();
        black_box(body);
    }
    report(
        "decision snapshot (1,000 variants, 200 profiles)",
        started,
        ITERATIONS,
    );
    remove_database(&path);
}

fn persistence_benchmark() {
    const ITERATIONS: usize = 5;
    let path = temp_database("persist");
    let _ = std::fs::remove_file(&path);
    fixture_database(&path);
    let app = router(path.clone()).expect("load learning state");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("create benchmark runtime");
    let request_body = json!({
        "profile": profile_json(VARIANTS + 1),
        "specs": [{"key":"entry_threshold", "label":"Entry threshold", "default":0.5, "min":0.1, "max":1.0, "step":0.01, "unit":"score", "learning_direction":1.0}]
    }).to_string();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/profiles")
            .header("content-type", "application/json")
            .body(Body::from(request_body.clone()))
            .expect("build registration request");
        let response = runtime
            .block_on(app.clone().oneshot(request))
            .expect("profile registration request");
        black_box(response.status());
    }
    report(
        "learning state persistence (1,000 variants, 5,000 event IDs)",
        started,
        ITERATIONS,
    );
    remove_database(&path);
}

fn remove_database(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
}

fn main() {
    load_benchmark();
    snapshot_benchmark();
    persistence_benchmark();
}
