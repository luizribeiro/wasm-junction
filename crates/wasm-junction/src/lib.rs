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
mod component;
mod context;
mod middleware;
mod provider;

pub use app::{App, AppBuilder, BuildError, GetError, InterfaceHandle, LoadError, MissingImports};
pub use call::{Call, Caller, TypedCall};
pub use component::{Component, ComponentError};
pub use context::CallContext;
pub use middleware::{Event, Middleware, Next};
pub use provider::{Provided, Provider};
pub use wasm_junction_core::{
    BoxFuture, CallError, CallErrorKind, CompiledComponent, Engine, EngineError, Extensions,
    HostBound, ImportDispatcher, InvocationContext, MaybeSend, TypeError, Val, Vals,
};
