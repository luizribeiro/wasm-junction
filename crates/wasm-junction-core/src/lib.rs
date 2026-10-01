//! Engine contracts and engine-neutral component values for wasm-junction.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod context;
mod engine;
mod error;
mod future;
mod resource;
mod stream;
mod values;

pub use context::{Extensions, InvocationContext};
pub use engine::{
    CompiledComponent, Engine, EngineError, ImportDispatcher, ImportTarget, WasiConfig,
};
pub use error::{CallError, CallErrorKind};
pub use future::{BoxFuture, HostBound, MaybeSend};
pub use resource::{Resource, ResourceOwnership, ResourceTable};
pub use stream::{InputStream, OutputStream, OutputStreamWriter, StreamError, StreamHandle};
pub use values::{TypeError, Val, Vals};
