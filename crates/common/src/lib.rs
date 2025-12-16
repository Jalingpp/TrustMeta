pub mod boolean_expr;
pub mod rpc;
pub mod types;
pub mod verification;
pub mod config;

// Re-export commonly used types
pub use boolean_expr::{parse_boolean_expr, BooleanExpr};
pub use types::{AdsMode, Fid, Keyword, Proof, RootHash};
// Keep the canonical `SystemConfig` type from `types` (used throughout workspace)
pub use types::SystemConfig;
// Runtime-config loader (TOML/JSON) is `RuntimeConfig` in the `config` module
pub use config::RuntimeConfig;
pub use verification::ProofVerifier;
