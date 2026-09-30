use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::{Call, CallContext, Trap, Vals};

/// A boxed future that may move between threads on native targets.
#[cfg(not(target_arch = "wasm32"))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A boxed future that remains on the browser thread on WebAssembly targets.
#[cfg(target_arch = "wasm32")]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// The target-dependent bound for futures returned by generated host traits.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

/// The target-dependent bound for futures returned by generated host traits.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}

#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// The target-dependent bound for host implementations.
///
/// Native providers can be dispatched from multiple threads, so their state must be `Send` and
/// `Sync`. Browser providers stay on one thread and may contain values such as `Rc`.
///
/// ```compile_fail
/// # #[cfg(not(target_arch = "wasm32"))]
/// # {
/// use std::rc::Rc;
/// use wasm_junction::HostBound;
///
/// fn require_host<T: HostBound>() {}
/// require_host::<Rc<()>>();
/// # }
/// ```
#[cfg(not(target_arch = "wasm32"))]
pub trait HostBound: Send + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> HostBound for T {}

/// The target-dependent bound for host implementations.
///
/// WebAssembly providers remain on one thread and therefore have no `Send` or `Sync` bound.
#[cfg(target_arch = "wasm32")]
pub trait HostBound {}

#[cfg(target_arch = "wasm32")]
impl<T> HostBound for T {}

#[cfg(target_arch = "wasm32")]
const _: () = {
    fn accepts_host<T: HostBound>() {}
    let _ = accepts_host::<std::rc::Rc<()>>;
};

/// An object-safe implementation of one host or component interface.
pub trait Provider: HostBound {
    /// Invokes a function through its engine-neutral call representation.
    fn call<'a>(&'a self, cx: &'a CallContext, call: Call) -> BoxFuture<'a, Result<Vals, Trap>>;
}

/// A provider paired with the fully qualified interface it implements.
pub struct Provided {
    interface: &'static str,
    provider: Arc<dyn Provider>,
}

impl Provided {
    /// Pairs an implementation with its generated interface identifier for app registration.
    #[must_use]
    pub fn new(interface: &'static str, provider: impl Provider + 'static) -> Self {
        Self {
            interface,
            provider: Arc::new(provider),
        }
    }

    pub(crate) fn into_parts(self) -> (&'static str, Arc<dyn Provider>) {
        (self.interface, self.provider)
    }
}

impl std::fmt::Debug for Provided {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Provided")
            .field("interface", &self.interface)
            .field("provider", &Arc::as_ptr(&self.provider))
            .finish()
    }
}
