use wasm_junction_core::{
    CallError, EngineEvent, InvocationId, Resource as JunctionResource, ResourceOwnership, Val,
    Vals, validate_resource_for_invocation,
};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p2::bindings::clocks::wall_clock::Datetime;
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, DynPollable, IoError};

use super::trampoline::{self, Real};
use crate::engine::StoreData;

trait ToVal {
    fn to_val(self) -> Val;
}

trait FromVal: Sized {
    fn from_val(value: Val) -> Result<Self, CallError>;
}

trait WitResource: 'static {
    const INTERFACE: &'static str;
    const NAME: &'static str;
}

fn shape(expected: &str) -> CallError {
    CallError::trap(format!("expected {expected}"))
}

fn scope(value: Val, invocation: InvocationId) -> Val {
    match value {
        Val::Resource(resource) => Val::Resource(match resource.ownership() {
            ResourceOwnership::Own => JunctionResource::__owned_for_invocation(
                resource.interface(),
                resource.name(),
                resource.id(),
                invocation,
            ),
            ResourceOwnership::Borrow => JunctionResource::__borrowed_for_invocation(
                resource.interface(),
                resource.name(),
                resource.id(),
                invocation,
            ),
        }),
        Val::List(values) => Val::List(
            values
                .into_iter()
                .map(|value| scope(value, invocation))
                .collect(),
        ),
        value => value,
    }
}

fn scope_values(values: Vals, invocation: InvocationId) -> Vals {
    values
        .into_iter()
        .map(|value| scope(value, invocation))
        .collect()
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "validators share one fallible function signature in the gate macro"
)]
fn no_resource_validation(_values: &[Val], _store: &mut StoreData) -> Result<(), CallError> {
    Ok(())
}

fn validate_pollable_borrows(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    for value in values {
        match value {
            Val::List(values) => validate_pollable_borrows(values, store)?,
            Val::Resource(resource) => {
                validate_resource_for_invocation(
                    resource,
                    POLLABLE_INTERFACE,
                    POLLABLE,
                    ResourceOwnership::Borrow,
                    invocation,
                )?;
                store
                    .wasi_table()
                    .get(&Resource::<DynPollable>::new_borrow(resource.id()))
                    .map_err(|_| {
                        CallError::refused(format!("unknown pollable handle {}", resource.id()))
                    })?;
            }
            _ => return Err(shape(POLLABLE)),
        }
    }
    Ok(())
}
impl ToVal for String {
    fn to_val(self) -> Val {
        Val::String(self)
    }
}

impl FromVal for String {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::String(value) => Ok(value),
            _ => Err(shape("string")),
        }
    }
}

impl ToVal for u64 {
    fn to_val(self) -> Val {
        Val::U64(self)
    }
}

impl FromVal for u64 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::U64(value) => Ok(value),
            _ => Err(shape("u64")),
        }
    }
}

impl ToVal for bool {
    fn to_val(self) -> Val {
        Val::Bool(self)
    }
}

impl FromVal for bool {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Bool(value) => Ok(value),
            _ => Err(shape("bool")),
        }
    }
}

impl ToVal for u32 {
    fn to_val(self) -> Val {
        Val::U32(self)
    }
}

impl FromVal for u32 {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::U32(value) => Ok(value),
            _ => Err(shape("u32")),
        }
    }
}

const POLLABLE_INTERFACE: &str = "wasi:io/poll@0.2.12";
const POLLABLE: &str = "pollable";
const STREAMS_INTERFACE: &str = "wasi:io/streams@0.2.12";
const INPUT_STREAM: &str = "input-stream";
const OUTPUT_STREAM: &str = "output-stream";
const ERROR_INTERFACE: &str = "wasi:io/error@0.2.12";
const ERROR: &str = "error";

impl WitResource for DynPollable {
    const INTERFACE: &'static str = POLLABLE_INTERFACE;
    const NAME: &'static str = POLLABLE;
}

impl WitResource for DynInputStream {
    const INTERFACE: &'static str = STREAMS_INTERFACE;
    const NAME: &'static str = INPUT_STREAM;
}

impl WitResource for DynOutputStream {
    const INTERFACE: &'static str = STREAMS_INTERFACE;
    const NAME: &'static str = OUTPUT_STREAM;
}

impl WitResource for IoError {
    const INTERFACE: &'static str = ERROR_INTERFACE;
    const NAME: &'static str = ERROR;
}

impl<T: WitResource> ToVal for Resource<T> {
    fn to_val(self) -> Val {
        Val::Resource(if self.owned() {
            JunctionResource::owned(T::INTERFACE, T::NAME, self.rep())
        } else {
            JunctionResource::borrowed(T::INTERFACE, T::NAME, self.rep())
        })
    }
}

impl<T: WitResource> FromVal for Resource<T> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Resource(resource)
                if resource.interface() == T::INTERFACE && resource.name() == T::NAME =>
            {
                Ok(match resource.ownership() {
                    ResourceOwnership::Own => Self::new_own(resource.id()),
                    ResourceOwnership::Borrow => Self::new_borrow(resource.id()),
                })
            }
            _ => Err(shape(T::NAME)),
        }
    }
}

macro_rules! list_value {
    ($ty:ty) => {
        impl ToVal for Vec<$ty> {
            fn to_val(self) -> Val {
                Val::List(self.into_iter().map(ToVal::to_val).collect())
            }
        }

        impl FromVal for Vec<$ty> {
            fn from_val(value: Val) -> Result<Self, CallError> {
                match value {
                    Val::List(items) => items.into_iter().map(FromVal::from_val).collect(),
                    _ => Err(shape("list")),
                }
            }
        }
    };
}

list_value!(String);
list_value!((String, String));
list_value!(u32);
list_value!(Resource<DynPollable>);

impl ToVal for Vec<u8> {
    fn to_val(self) -> Val {
        Val::Bytes(self)
    }
}

impl FromVal for Vec<u8> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Bytes(bytes) => Ok(bytes),
            _ => Err(shape("bytes")),
        }
    }
}

impl<T: ToVal> ToVal for Option<T> {
    fn to_val(self) -> Val {
        Val::Option(self.map(|value| Box::new(value.to_val())))
    }
}

impl<T: FromVal> FromVal for Option<T> {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Option(value) => value.map(|value| T::from_val(*value)).transpose(),
            _ => Err(shape("option")),
        }
    }
}

impl<A: ToVal, B: ToVal> ToVal for (A, B) {
    fn to_val(self) -> Val {
        Val::Tuple(vec![self.0.to_val(), self.1.to_val()])
    }
}

impl<A: FromVal, B: FromVal> FromVal for (A, B) {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Tuple(fields) = value else {
            return Err(shape("tuple"));
        };
        let [first, second] = <[Val; 2]>::try_from(fields).map_err(|_| shape("pair"))?;
        Ok((A::from_val(first)?, B::from_val(second)?))
    }
}

impl ToVal for Datetime {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("seconds".to_owned(), Val::U64(self.seconds)),
            ("nanoseconds".to_owned(), Val::U32(self.nanoseconds)),
        ])
    }
}

impl FromVal for Datetime {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("datetime"));
        };
        match fields.as_slice() {
            [(seconds, Val::U64(value)), (nanoseconds, Val::U32(nanos))]
                if seconds == "seconds" && nanoseconds == "nanoseconds" =>
            {
                Ok(Self {
                    seconds: *value,
                    nanoseconds: *nanos,
                })
            }
            _ => Err(shape("datetime fields")),
        }
    }
}

fn finish<T: FromVal>(outcome: Result<Vals, CallError>) -> wasmtime::Result<T> {
    let values = outcome.map_err(wasmtime::Error::new)?;
    let [value] = <[Val; 1]>::try_from(values).map_err(|_| shape("one result"))?;
    T::from_val(value).map_err(wasmtime::Error::new)
}

fn finish_unit(outcome: Result<Vals, CallError>) -> wasmtime::Result<()> {
    let values = outcome.map_err(wasmtime::Error::new)?;
    if values.is_empty() {
        Ok(())
    } else {
        Err(wasmtime::Error::new(shape("no results")))
    }
}

macro_rules! gate {
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, plain,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, no_resource_validation, , $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, resource,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, no_resource_validation, , $signature -> $ok, one);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, borrowed,
     ($($arg:ident: $ty:ty),*) -> ()) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_pollable_borrows, await,
            ($($arg: $ty),*) -> (), unit);
    };
    ($linker:ident, $iface:literal, $name:literal, $view:ident, $method:path, borrowed,
     $signature:tt -> $ok:ty) => {
        gate!(@define $linker, $iface, $name, $view, $method, validate_pollable_borrows, await,
            $signature -> $ok, one);
    };
    (@define $linker:ident, $iface:literal, $name:literal, $view:ident, $method:path,
     $validate:ident, $($await:ident)?, ($($arg:ident: $ty:ty),*) -> $ok:ty, $shape:ident) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = scope_values(vec![$($arg.to_val()),*], invocation);
                let real: Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| shape("another argument"))?
                    )?;)*
                    let value = $method(&mut views::$view(store.data_mut()) $(, $arg)*) $(.$await)?
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(scope_values(gate!(@values value, $shape), invocation))
                });
                let outcome = trampoline::gate(&mut store, $iface, $name, args, real).await;
                gate!(@return outcome, $ok, $shape)
            }),
        )?;
    };
    (@values $value:ident, one) => { vec![$value.to_val()] };
    (@values $value:ident, unit) => {{ let _ = $value; Vec::new() }};
    (@return $outcome:ident, $ok:ty, one) => { Ok((finish::<$ok>($outcome)?,)) };
    (@return $outcome:ident, $ok:ty, unit) => {{ finish_unit($outcome)?; Ok(()) }};
}

macro_rules! gate_drop {
    ($linker:ident, $iface:ident, $name:ident, $drop:literal, $ty:ty, $direction:expr,
     $method:path $(, $await:ident)?) => {
        $linker.instance($iface)?.resource_async(
            $name,
            wasmtime::component::ResourceType::host::<$ty>(),
            |mut store, id| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI drop has no invocation id"))?;
                let resource = JunctionResource::__owned_for_invocation(
                    $iface, $name, id, invocation,
                );
                store.data().imports.emit(EngineEvent::ResourceDrop {
                    invocation,
                    resource: resource.clone(),
                });
                let real: Real = |mut store, args| Box::pin(async move {
                    let [Val::Resource(resource)] = <[Val; 1]>::try_from(args)
                        .map_err(|_| shape($name))?
                    else {
                        return Err(shape($name));
                    };
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI drop has no invocation id"))?;
                    validate_resource_for_invocation(
                        &resource, $iface, $name, ResourceOwnership::Own, invocation,
                    )?;
                    $method(views::io(store.data_mut()), Resource::<$ty>::new_own(resource.id()))
                        $(.$await)?
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(Vec::new())
                });
                let values = trampoline::gate(
                    &mut store,
                    $iface,
                    $drop,
                    vec![Val::Resource(resource)],
                    real,
                )
                .await
                .map_err(wasmtime::Error::new)?;
                if !values.is_empty() {
                    return Err(wasmtime::Error::new(shape("no results")));
                }
                Ok(())
            }),
        )?;
    };
}

pub(super) fn add_environment(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:cli/environment@0.2.12", "get-environment", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::get_environment,
        plain, () -> Vec<(String, String)>);
    gate!(linker, "wasi:cli/environment@0.2.12", "get-arguments", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::get_arguments,
        plain, () -> Vec<String>);
    gate!(linker, "wasi:cli/environment@0.2.12", "initial-cwd", cli,
        wasmtime_wasi::p2::bindings::cli::environment::Host::initial_cwd,
        plain, () -> Option<String>);
    Ok(())
}

pub(super) fn add_wall_clock(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:clocks/wall-clock@0.2.12", "now", clocks,
        wasmtime_wasi::p2::bindings::clocks::wall_clock::Host::now,
        plain, () -> Datetime);
    gate!(linker, "wasi:clocks/wall-clock@0.2.12", "resolution", clocks,
        wasmtime_wasi::p2::bindings::clocks::wall_clock::Host::resolution,
        plain, () -> Datetime);
    Ok(())
}

pub(super) fn add_monotonic_clock(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "now", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::now,
        plain, () -> u64);
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "subscribe-instant", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::subscribe_instant,
        resource, (when: u64) -> Resource<DynPollable>);
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "subscribe-duration", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::subscribe_duration,
        resource, (duration: u64) -> Resource<DynPollable>);
    gate!(linker, "wasi:clocks/monotonic-clock@0.2.12", "resolution", clocks,
        wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host::resolution,
        plain, () -> u64);
    Ok(())
}

pub(super) fn add_poll(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        POLLABLE_INTERFACE,
        POLLABLE,
        "[drop]pollable",
        DynPollable,
        None,
        wasmtime_wasi::p2::bindings::io::poll::HostPollable::drop
    );
    gate!(linker, "wasi:io/poll@0.2.12", "[method]pollable.ready", io,
        wasmtime_wasi::p2::bindings::io::poll::HostPollable::ready,
        borrowed, (pollable: Resource<DynPollable>) -> bool);
    gate!(linker, "wasi:io/poll@0.2.12", "[method]pollable.block", io,
        wasmtime_wasi::p2::bindings::io::poll::HostPollable::block,
        borrowed, (pollable: Resource<DynPollable>) -> ());
    gate!(linker, "wasi:io/poll@0.2.12", "poll", io,
        wasmtime_wasi::p2::bindings::io::poll::Host::poll,
        borrowed, (pollables: Vec<Resource<DynPollable>>) -> Vec<u32>);
    Ok(())
}

mod views {
    use wasmtime_wasi::cli::WasiCliView;
    use wasmtime_wasi::clocks::WasiClocksView;

    use crate::engine::StoreData;

    pub(super) fn cli(store: &mut StoreData) -> wasmtime_wasi::cli::WasiCliCtxView<'_> {
        store.cli()
    }

    pub(super) fn clocks(store: &mut StoreData) -> wasmtime_wasi::clocks::WasiClocksCtxView<'_> {
        store.clocks()
    }

    pub(super) fn io(store: &mut StoreData) -> &mut wasmtime::component::ResourceTable {
        store.wasi_table()
    }
}
