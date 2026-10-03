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

    /// Registers the selected engine's WASI Preview 3 outgoing HTTP implementation.
    ///
    /// Use this together with [`super::provider`] for components that import
    /// `wasi:http/client@0.3.0` or `wasi:http/types@0.3.0`. An engine without outgoing HTTP
    /// support returns [`BuildError::UnsupportedEngineProvider`](crate::BuildError::UnsupportedEngineProvider).
    ///
    /// Middleware sees `client.send` arguments as the owned request handle followed by method,
    /// optional scheme, optional authority, optional path-with-query, and a list of `(name,
    /// bytes)` headers. Changes to those metadata arguments are applied to the request before
    /// network I/O starts.
    #[must_use]
    pub const fn provider() -> Provided {
        Provided::engine(WASI_HTTP_PROVIDER_NAME)
    }
}
