//! OpenWorkers runtime backed by the Nova JavaScript engine.
//!
//! Status: placeholder — see README.md. The `Worker` type exists so the
//! crate compiles against `openworkers-core` v0.14 from day one; every
//! method is `todo!()` until the Nova integration starts.

use openworkers_core::{Event, RuntimeLimits, Script, TerminationReason};

// Re-export the engine so downstream experiments don't need a direct dep.
pub use nova_vm;

pub struct Worker;

impl openworkers_core::Worker for Worker {
    async fn new(
        script: Script,
        limits: Option<RuntimeLimits>,
    ) -> Result<Self, TerminationReason> {
        let _ = (script, limits);

        todo!("Nova runtime is not implemented yet");
    }

    async fn exec(&mut self, task: Event) -> Result<(), TerminationReason> {
        let _ = task;

        todo!("Nova runtime is not implemented yet");
    }

    fn abort(&mut self) {}
}
