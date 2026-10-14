use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct PocketFitExtension {
    pub loaded: bool,
    pub engine: String,
    pub fit: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiModel {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: &'static str,
    pub pocket: PocketFitExtension,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelsListResponse {
    pub object: &'static str,
    pub data: Vec<OpenAiModel>,
}

pub trait InstalledModels: Send + Sync {
    fn list(&self) -> Vec<OpenAiModel>;
}

pub fn router(models: Arc<dyn InstalledModels>) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .with_state(models)
}

async fn list_models(State(models): State<Arc<dyn InstalledModels>>) -> Json<ModelsListResponse> {
    Json(ModelsListResponse {
        object: "list",
        data: models.list(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    struct FixtureModels;
    impl InstalledModels for FixtureModels {
        fn list(&self) -> Vec<OpenAiModel> {
            vec![OpenAiModel {
                id: "mlx-community/Qwen3-4B-Instruct-2507-4bit@a1b2c3d".into(),
                object: "model",
                created: 1760400000,
                owned_by: "pocketai",
                pocket: PocketFitExtension {
                    loaded: false,
                    engine: "mlx".into(),
                    fit: "runs_well".into(),
                },
            }]
        }
    }

    /// Golden test vs contracts/openai/models.list.json: every field that
    /// fixture declares must match exactly; `pocket` is additive (OpenAI
    /// clients that don't know about it simply ignore the extra field).
    #[tokio::test]
    async fn matches_the_openai_models_list_golden_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/openai/models.list.json"
        ))
        .unwrap();

        let app = router(Arc::new(FixtureModels));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(actual["object"], fixture["object"]);
        let actual_model = &actual["data"][0];
        let fixture_model = &fixture["data"][0];
        for field in ["id", "object", "created", "owned_by"] {
            assert_eq!(
                actual_model[field], fixture_model[field],
                "field {field} diverged from the golden fixture"
            );
        }
        assert!(actual_model["pocket"]["loaded"].is_boolean());
    }
}
