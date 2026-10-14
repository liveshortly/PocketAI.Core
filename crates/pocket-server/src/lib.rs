mod auth;
mod devices;
mod events;
mod lifecycle;

pub use devices::DeviceService;
pub use events::{Event, EventBus};
pub use auth::{
    authenticate, openai_error_json, pocket_error_json, sha256_hex, AuthError, AuthStore,
    DeviceRateLimiter, Identity, LockoutTracker,
};
pub use lifecycle::{start, ServerConfig, ServerHandle};
