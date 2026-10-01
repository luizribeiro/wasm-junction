//! Engine contracts and engine-neutral component values for wasm-junction.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod context;
mod engine;
mod error;
mod future;
mod values;

pub use context::{Extensions, InvocationContext};
pub use engine::{CompiledComponent, Engine, EngineError, ImportDispatcher};
pub use error::{CallError, CallErrorKind};
pub use future::{BoxFuture, HostBound, MaybeSend};
pub use values::{TypeError, Val, Vals};
