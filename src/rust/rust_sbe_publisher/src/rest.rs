use axum::{Json, Router, extract::Path, http::StatusCode, routing::get};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub async fn serve() -> anyhow::Result<()> {
    let users = Arc::new(create_users(10_000));

    let app = Router::new().route(
        "/users/{page}",
        get({
            let users = Arc::clone(&users);
            move |path| get_user_page(path, users)
        }),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_user_page(
    Path(page): Path<i32>,
    users: Arc<Vec<User>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let request_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let page_size: usize = 100;
    let start = page as usize * page_size;
    let end = start + page_size;

    if start >= users.len() {
        return Err((StatusCode::NOT_FOUND, "Page exceeds total length".into()));
    }

    let number_of_items = if end > users.len() {
        users.len() - start
    } else {
        page_size
    };

    let has_more = number_of_items == page_size && start + page_size < users.len();
    let users = users
        .iter()
        .skip(start)
        .take(number_of_items)
        .map(|user| UserResponse {
            id: user.id,
            last_logged_in: fake_last_logged_in(user.id, request_time.as_secs()),
        })
        .collect::<Vec<_>>();

    Ok(Json(
        serde_json::to_value(Response {
            data: users,
            has_more,
            error: None,
        })
        .unwrap(),
    ))
}

fn create_users(count: u64) -> Vec<User> {
    (0..count).map(|id| User { id }).collect()
}

fn fake_last_logged_in(user_id: u64, now: u64) -> u64 {
    const THIRTY_DAYS_SECONDS: u64 = 30 * 24 * 60 * 60;
    const UPDATE_INTERVAL_SECONDS: u64 = 20;

    let phase_offset = user_phase_offset(user_id);
    let update_slot = now.saturating_add(phase_offset) / UPDATE_INTERVAL_SECONDS;
    let slot_start = update_slot * UPDATE_INTERVAL_SECONDS - phase_offset;
    let mut hasher = DefaultHasher::new();
    user_id.hash(&mut hasher);
    update_slot.hash(&mut hasher);
    slot_start.saturating_sub(hasher.finish() % THIRTY_DAYS_SECONDS)
}

fn user_phase_offset(user_id: u64) -> u64 {
    const UPDATE_INTERVAL_SECONDS: u64 = 10;

    let mut hasher = DefaultHasher::new();
    user_id.hash(&mut hasher);
    hasher.finish() % UPDATE_INTERVAL_SECONDS
}

#[cfg(test)]
mod tests {
    use super::{fake_last_logged_in, user_phase_offset};

    #[test]
    fn fake_login_times_are_stable_for_ten_seconds_and_staggered_across_users() {
        let now = 1_800_000_100;
        let first_window_start = (now + user_phase_offset(0)) / 10 * 10 - user_phase_offset(0);
        let first_window = fake_last_logged_in(0, first_window_start);
        assert_eq!(first_window, fake_last_logged_in(0, first_window_start + 9));
        assert_ne!(
            first_window,
            fake_last_logged_in(0, first_window_start + 10)
        );

        let phases = (0..100)
            .map(user_phase_offset)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(phases.len(), 10);
    }
}

#[derive(Clone, Debug)]
struct User {
    id: u64,
}

#[derive(Clone, Debug, Serialize)]
struct UserResponse {
    id: u64,
    last_logged_in: u64,
}

#[derive(Clone, Debug, Serialize)]
struct Response {
    data: Vec<UserResponse>,
    has_more: bool,
    error: Option<String>,
}
