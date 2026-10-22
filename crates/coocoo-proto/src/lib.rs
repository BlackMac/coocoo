//! Shared contract for every Coocoo crate and client.
//!
//! - [`wire`]: request/response/event envelopes exchanged over `/ws`.
//! - [`types`]: domain objects (hosts, projects, agents, timeline, browser).
//! - [`bus`]: in-process event fan-out ([`Emitter`]) and per-connection sinks ([`ConnSink`]).
//! - [`ApiError`]: the only error type that crosses a subsystem boundary.
//! - [`Orchestrator`]: routed calls back into the daemon (local or peer host).
//! - [`paths`], [`util`]: storage locations, atomic JSON persistence, ids, clocks.
//!
//! Subsystem crates (`coocoo-agents`, `coocoo-browser`, `coocoo-workspace`) depend only on this
//! crate; `coocoo-core` wires them together.

pub mod bus;
pub mod error;
pub mod orchestrator;
pub mod paths;
pub mod types;
pub mod util;
pub mod wire;

pub use bus::{ConnSink, Emitter};
pub use error::{ApiError, ApiResult};
pub use orchestrator::Orchestrator;
pub use paths::Paths;
pub use types::*;
pub use wire::*;

/// Protocol version advertised in `system.hello`; bump on breaking wire changes.
pub const PROTOCOL_VERSION: u32 = 1;
