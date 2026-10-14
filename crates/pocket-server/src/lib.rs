mod auth;
mod lifecycle;

pub use auth::{
    authenticate, openai_error_json, pocket_error_json, sha256_hex, AuthError, AuthStore,
    DeviceRateLimiter, Identity, LockoutTracker,
};
pub use lifecycle::{start, ServerConfig, ServerHandle};
