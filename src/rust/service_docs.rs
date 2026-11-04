// Shared Axum routes used by each HTTP service to expose its OpenAPI document
// and a Swagger UI backed by that document.
use axum::{Router, response::Html, routing::get};

pub fn routes(
    service: &'static str,
    paths: &'static [(&'static str, &'static [&'static str])],
) -> Router {
    let document = serde_json::json!({
        "openapi": "3.0.3",
        "info": { "title": service, "version": "1.0.0" },
        "paths": paths.iter().map(|(path, methods)| ((*path).to_string(),
            serde_json::Value::Object(methods.iter().map(|method| ((*method).to_string(), serde_json::json!({
                "summary": format!("{service} {method} {path}"),
                "responses": { "200": { "description": "Successful response" } }
            }))).collect::<serde_json::Map<String, serde_json::Value>>())
        )).collect::<serde_json::Map<String, serde_json::Value>>()
    });
    Router::new()
        .route("/openapi.json", get(move || { let document = document.clone(); async move { axum::Json(document) } }))
        .route("/docs", get(|| async { Html(r#"<!doctype html><html><head><title>Swagger UI</title><link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css"></head><body><div id="swagger-ui"></div><script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script><script>SwaggerUIBundle({url:'/openapi.json',dom_id:'#swagger-ui'});</script></body></html>"#) }))
}
