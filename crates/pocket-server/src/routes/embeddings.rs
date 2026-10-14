use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    One(String),
    Many(Vec<String>),
}

impl EmbeddingInput {
    fn into_vec(self) -> Vec<String> {
        match self {
            EmbeddingInput::One(s) => vec![s],
            EmbeddingInput::Many(v) => v,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsRequest {
    pub model: String,
    pub input: EmbeddingInput,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingObject {
    pub object: &'static str,
    pub index: usize,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingsResponse {
    pub object: &'static str,
    pub data: Vec<EmbeddingObject>,
    pub model: String,
    pub usage: Usage,
}

pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, model: &str, inputs: &[String]) -> Vec<Vec<f32>>;
}

pub fn router(provider: Arc<dyn EmbeddingProvider>) -> Router {
    Router::new()
        .route("/v1/embeddings", post(create_embeddings))
        .with_state(provider)
}

async fn create_embeddings(
    State(provider): State<Arc<dyn EmbeddingProvider>>,
    Json(req): Json<EmbeddingsRequest>,
) -> Json<EmbeddingsResponse> {
    let inputs = req.input.into_vec();
    let prompt_tokens: u32 = inputs
        .iter()
        .map(|s| s.split_whitespace().count() as u32)
        .sum();
    let vectors = provider.embed(&req.model, &inputs);

    Json(EmbeddingsResponse {
        object: "list",
        data: vectors
            .into_iter()
            .enumerate()
            .map(|(index, embedding)| EmbeddingObject {
                object: "embedding",
                index,
                embedding,
            })
            .collect(),
        model: req.model,
        usage: Usage {
            prompt_tokens,
            total_tokens: prompt_tokens,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    struct FixedProvider;
    impl EmbeddingProvider for FixedProvider {
        fn embed(&self, _model: &str, inputs: &[String]) -> Vec<Vec<f32>> {
            inputs
                .iter()
                .map(|_| vec![0.012, -0.034, 0.056, 0.078])
                .collect()
        }
    }

    /// Golden test vs contracts/openai/embeddings.{req,res}.json: the
    /// request fixture parses into our request type, and the response we
    /// produce has the same top-level shape as the response fixture.
    #[tokio::test]
    async fn matches_the_openai_embeddings_golden_fixtures() {
        let req_fixture = include_str!("../../../../contracts/openai/embeddings.req.json");
        let res_fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/openai/embeddings.res.json"
        ))
        .unwrap();

        let app = router(Arc::new(FixedProvider));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(req_fixture.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(actual["object"], res_fixture["object"]);
        assert_eq!(
            actual["data"][0]["object"],
            res_fixture["data"][0]["object"]
        );
        assert!(actual["data"][0]["embedding"].is_array());
        assert!(actual["usage"]["prompt_tokens"].is_number());
        assert!(actual["usage"]["total_tokens"].is_number());
    }

    #[tokio::test]
    async fn accepts_a_single_string_input_not_just_an_array() {
        let app = router(Arc::new(FixedProvider));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"bge-small","input":"hello world"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(actual["data"].as_array().unwrap().len(), 1);
    }
}
