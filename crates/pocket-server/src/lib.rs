mod auth;
mod devices;
mod events;
mod lifecycle;
pub mod routes;

pub use auth::{
    authenticate, openai_error_json, pocket_error_json, require_auth, sha256_hex, AuthError,
    AuthStore, DeviceRateLimiter, Identity, LockoutTracker,
};
pub use devices::DeviceService;
pub use events::{Event, EventBus};
pub use lifecycle::{cert_fingerprint, start, start_with_routes, ServerConfig, ServerHandle};
pub use routes::agents::{AgentsState, SessionEntry, SessionStatus, SharedAgents};
pub use routes::embeddings::EmbeddingProvider;
pub use routes::models::{InstalledModels, ModelsListResponse, OpenAiModel, PocketFitExtension};
