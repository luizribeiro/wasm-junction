//! WASI providers implemented by the selected component engine.
//!
#![cfg_attr(
    not(feature = "wasi-http"),
    doc = "Outgoing HTTP is absent unless its feature is enabled:\n\n```compile_fail\nlet _ = wasm_junction::wasi::http::provider();\n```"
)]

use crate::{Provided, WASI_PROVIDER_NAME};

/// Registers the selected engine's WASI Preview 2 implementation.
///
/// Use this when components import WASI. Without it, loading such a component reports
/// [`MissingImports`](crate::MissingImports). Registering it twice, or alongside an application
/// provider for the same interface, returns [`BuildError::DuplicateProvider`](crate::BuildError::DuplicateProvider).
/// An engine without WASI returns
/// [`BuildError::UnsupportedEngineProvider`](crate::BuildError::UnsupportedEngineProvider).
///
/// Preview 2 and Preview 3 filesystem descriptor methods expose their declared WIT arguments
/// first, followed by the guest preopen path for each descriptor argument in the same order, with
/// the receiver first. Descriptor drops likewise place their preopen path after the owned handle.
/// A Preview 2 directory-entry-stream read places its preopen path after the borrowed stream
/// handle. This appended context identifies the configured capability that produced a handle;
/// middleware should make policy decisions from it rather than from relative path arguments.
///
/// Socket calls expose declared WIT arguments first. Addresses on connect, bind, UDP stream, and
/// datagram send calls therefore keep their declared position, as does the name on
/// `resolve-addresses`. Handle-only calls append one policy value per socket-derived handle in
/// handle-argument order: an `ip-socket-address` value for TCP sockets, byte streams, UDP sockets,
/// and datagram streams, or the queried string for resolve streams. A connected handle uses its
/// peer address; an unconnected UDP handle uses its local address. Resource drops use the owned
/// handle followed by the same policy value.
///
/// ```
/// use wasm_junction::{App, BuildError, wasi};
///
/// # fn build() -> Result<(), BuildError> {
/// let app = App::builder().provide(wasi::provider()).build()?;
/// # let _ = app;
/// # Ok(())
/// # }
/// ```
#[must_use]
pub const fn provider() -> Provided {
    Provided::engine(WASI_PROVIDER_NAME)
}

/// Outgoing HTTP support implemented by the selected component engine.
#[cfg(feature = "wasi-http")]
pub mod http {
    use crate::{Provided, WASI_HTTP_PROVIDER_NAME};

    /// Registers the selected engine's WASI outgoing HTTP implementation.
    ///
    /// Use this together with [`super::provider`] for components that import
    /// `wasi:http/outgoing-handler@0.2.12` or, with the `wasi-p3` feature,
    /// `wasi:http/client@0.3.0`. An engine without outgoing HTTP support returns
    /// [`BuildError::UnsupportedEngineProvider`](crate::BuildError::UnsupportedEngineProvider).
    ///
    /// Middleware sees `outgoing-handler.handle` and `client.send` arguments as the owned request
    /// handle followed by method, optional scheme, optional authority, optional path-with-query,
    /// and a list of `(name, bytes)` headers. Preview 2 `handle` then includes its declared
    /// optional request-options argument. Changes to metadata are applied before network I/O.
    #[must_use]
    pub const fn provider() -> Provided {
        Provided::engine(WASI_HTTP_PROVIDER_NAME)
    }
}
