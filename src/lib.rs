//! OpenWorkers runtime backed by the Nova JavaScript engine.
//!
//! v0 scope: synchronous fetch handlers only (no host I/O from JS yet).
//! See NOTES-nova-api.md for the engine reconnaissance and README.md for
//! the current status.

mod worker;

pub use worker::Worker;

// Re-export the common types and the engine so downstream code doesn't
// need direct dependencies.
pub use openworkers_core;

pub use nova_vm;
