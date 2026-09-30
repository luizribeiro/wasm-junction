use std::future::Future;
use std::pin::Pin;

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
