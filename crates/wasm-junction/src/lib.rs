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
#[cfg(feature = "wasi")]
pub mod wasi;

pub use app::{
    App, AppBuilder, BuildError, Candidate, CheckError, ConfigureError, GetError, Handle,
    InterfaceHandle, IssueKind, LinkError, LoadError, MissingImports, ReloadError, ResolutionIssue,
    UnloadError,
};
pub use call::{Call, Caller, TypedCall};
pub use component::{Component, ComponentError};
pub use context::CallContext;
pub use middleware::{Event, Middleware, Next};
pub use provider::{Provided, Provider};
pub use wasm_junction_core::{
    BoxFuture, CallError, CallErrorKind, ChannelDirection, CompiledComponent, Engine, EngineError,
    Extensions, HostBound, ImportDispatcher, ImportTarget, InputStream, InvocationContext,
    InvocationId, MaybeSend, OutputStream, OutputStreamWriter, Resource, ResourceOwnership,
    ResourceTable, StreamError, StreamHandle, TypeError, Val, Vals, WASI_PROVIDER_NAME,
    WasiSettings, validate_resource_for_invocation,
};
/// Generates bindings for every interface in a local WIT package.
///
/// Generated WIT errors display enum and payload-free variant cases by their
/// kebab-case names. Variant payloads follow the case name, while record
/// errors display declaration-ordered `field: value` pairs. Values use
/// [`Display`](std::fmt::Display) when available and compact debug output otherwise.
///
/// Typed handles use the interface's `UpperCamelCase` name. When that name is reserved by the
/// generated surface, `Handle` is appended: `host` generates `host::HostHandle`, for example.
///
/// Futures, non-byte streams, and byte streams nested beyond one `option` or `result` are
/// rejected until their runtime support is available.
///
/// ```compile_fail
/// wasm_junction::bindgen!({ path: "tests/fixtures/unsupported/wit" });
/// ```
pub use wasm_junction_macros::bindgen;
