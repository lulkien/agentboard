//! The SurrealDB backend, compiled only under the `surreal` feature. This step
//! is the seam only: every operation returns the stub error. Connection, serve
//! and the write lock arrive in later steps (docs/surrealdb-migration.md).

// The client crate is not called yet; this keeps the optional dependency honest.
use surrealdb as _;

/// Replaced by the real implementation as the ports land.
pub use super::stub::StubBoard as SurrealBoard;
