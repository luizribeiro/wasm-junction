//! Engine-neutral building blocks for applications composed from WebAssembly components.
//!
//! The API is intentionally small while the framework is under active development.
//!
//! ```
//! use wasm_junction as _;
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod call;
mod context;
mod engine;
mod middleware;
mod provider;
mod values;

pub use app::{App, AppBuilder, BuildError};
pub use call::{Call, Caller, Trap, TypedCall};
pub use context::{CallContext, Extensions};
pub use engine::{CompiledComponent, Engine, ImportDispatcher};
pub use middleware::{Event, Middleware, Next};
pub use provider::{BoxFuture, HostBound, MaybeSend, Provided, Provider};
pub use values::{TypeError, Val, Vals};
