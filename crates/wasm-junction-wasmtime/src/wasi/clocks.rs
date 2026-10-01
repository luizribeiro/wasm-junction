use std::sync::{Arc, Mutex};

use wasm_junction_core::{BoxFuture, CallError, ImportTarget, InvocationContext, Val, Vals};
use wasmtime::component::{Linker, Resource};
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi::clocks::WasiClocksView;
use wasmtime_wasi::p2::DynPollable;
use wasmtime_wasi::p2::bindings::clocks::{monotonic_clock, wall_clock};

use super::{StoreData, WasiState, dispatch, lock};
use crate::GATED_WASI_INTERFACES;

const MONOTONIC_INTERFACE: &str = GATED_WASI_INTERFACES[1];
const WALL_INTERFACE: &str = GATED_WASI_INTERFACES[2];

#[derive(Clone, Copy)]
enum WallOperation {
    Now,
    Resolution,
}

struct WallTarget(Arc<Mutex<WasiState>>, WallOperation);

impl ImportTarget for WallTarget {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        let state = self.0.clone();
        let operation = self.1;
        Box::pin(async move {
            if !args.is_empty() {
                return Err(CallError::trap("WASI wall-clock call takes no arguments"));
            }
            let mut state = lock(&state);
            let datetime = match operation {
                WallOperation::Now => wall_clock::Host::now(&mut state.clocks()),
                WallOperation::Resolution => wall_clock::Host::resolution(&mut state.clocks()),
            }
            .map_err(|error| CallError::trap(error.to_string()))?;
            Ok(vec![Val::Record(vec![
                ("seconds".to_owned(), Val::U64(datetime.seconds)),
                ("nanoseconds".to_owned(), Val::U32(datetime.nanoseconds)),
            ])])
        })
    }
}

#[derive(Clone, Copy)]
enum MonotonicOperation {
    Now,
    Resolution,
}

struct MonotonicTarget(Arc<Mutex<WasiState>>, MonotonicOperation);

impl ImportTarget for MonotonicTarget {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        let state = self.0.clone();
        let operation = self.1;
        Box::pin(async move {
            if !args.is_empty() {
                return Err(CallError::trap("WASI monotonic read takes no arguments"));
            }
            let mut state = lock(&state);
            let value = match operation {
                MonotonicOperation::Now => monotonic_clock::Host::now(&mut state.clocks()),
                MonotonicOperation::Resolution => {
                    monotonic_clock::Host::resolution(&mut state.clocks())
                }
            }
            .map_err(|error| CallError::trap(error.to_string()))?;
            Ok(vec![Val::U64(value)])
        })
    }
}

#[derive(Clone, Copy)]
enum Subscription {
    Instant,
    Duration,
}

struct SubscriptionTarget;

impl ImportTarget for SubscriptionTarget {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        Box::pin(async move {
            let [Val::U64(value)] = args.as_slice() else {
                return Err(CallError::trap("WASI clock subscription expects one u64"));
            };
            Ok(vec![Val::U64(*value)])
        })
    }
}

pub(super) fn add_wall_clock_gate(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    let mut instance = linker.instance(WALL_INTERFACE)?;
    instance.func_wrap_async("now", |mut store, (): ()| {
        Box::new(async move { Ok((call_wall(&mut store, "now", WallOperation::Now).await?,)) })
    })?;
    instance.func_wrap_async("resolution", |mut store, (): ()| {
        Box::new(async move {
            Ok((call_wall(&mut store, "resolution", WallOperation::Resolution).await?,))
        })
    })?;
    add_monotonic_clock_gate(linker)
}

fn add_monotonic_clock_gate(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    let mut instance = linker.instance(MONOTONIC_INTERFACE)?;
    instance.func_wrap_async("now", |mut store, (): ()| {
        Box::new(
            async move { Ok((call_monotonic(&mut store, "now", MonotonicOperation::Now).await?,)) },
        )
    })?;
    instance.func_wrap_async("resolution", |mut store, (): ()| {
        Box::new(async move {
            Ok(
                (
                    call_monotonic(&mut store, "resolution", MonotonicOperation::Resolution)
                        .await?,
                ),
            )
        })
    })?;
    instance.func_wrap_async("subscribe-instant", |mut store, (value,): (u64,)| {
        Box::new(async move { Ok((subscribe(&mut store, Subscription::Instant, value).await?,)) })
    })?;
    instance.func_wrap_async("subscribe-duration", |mut store, (value,): (u64,)| {
        Box::new(async move { Ok((subscribe(&mut store, Subscription::Duration, value).await?,)) })
    })
}

async fn call_wall(
    store: &mut StoreContextMut<'_, StoreData>,
    function: &'static str,
    operation: WallOperation,
) -> wasmtime::Result<wall_clock::Datetime> {
    let target = {
        let mut context = store.as_context_mut();
        let data = context.data_mut();
        Arc::new(WallTarget(data.gated_wasi.clone(), operation))
    };
    let values = dispatch(store, WALL_INTERFACE, function, Vec::new(), target).await?;
    decode_datetime(&values)
}

fn decode_datetime(values: &[Val]) -> wasmtime::Result<wall_clock::Datetime> {
    let [Val::Record(fields)] = values else {
        return Err(wasmtime::Error::msg("wall clock returned the wrong shape"));
    };
    match fields.as_slice() {
        [(seconds, Val::U64(value)), (nanoseconds, Val::U32(nanos))]
            if seconds == "seconds" && nanoseconds == "nanoseconds" =>
        {
            Ok(wall_clock::Datetime {
                seconds: *value,
                nanoseconds: *nanos,
            })
        }
        _ => Err(wasmtime::Error::msg(
            "wall clock record has the wrong shape",
        )),
    }
}

async fn call_monotonic(
    store: &mut StoreContextMut<'_, StoreData>,
    function: &'static str,
    operation: MonotonicOperation,
) -> wasmtime::Result<u64> {
    let target = {
        let mut context = store.as_context_mut();
        let data = context.data_mut();
        Arc::new(MonotonicTarget(data.gated_wasi.clone(), operation))
    };
    let values = dispatch(store, MONOTONIC_INTERFACE, function, Vec::new(), target).await?;
    match values.as_slice() {
        [Val::U64(value)] => Ok(*value),
        _ => Err(wasmtime::Error::msg(
            "monotonic clock returned the wrong shape",
        )),
    }
}

async fn subscribe(
    store: &mut StoreContextMut<'_, StoreData>,
    operation: Subscription,
    value: u64,
) -> wasmtime::Result<Resource<DynPollable>> {
    let function = match operation {
        Subscription::Instant => "subscribe-instant",
        Subscription::Duration => "subscribe-duration",
    };
    let values = dispatch(
        store,
        MONOTONIC_INTERFACE,
        function,
        vec![Val::U64(value)],
        Arc::new(SubscriptionTarget),
    )
    .await?;
    let [Val::U64(value)] = values.as_slice() else {
        return Err(wasmtime::Error::msg(
            "clock subscription returned the wrong shape",
        ));
    };

    // Clock reads can be faked completely because their values fit in `Val`. A pollable
    // resource does not, so subscriptions gate only their scalar input before resolving
    // against the real clock and primary resource table.
    let mut context = store.as_context_mut();
    let data = context.data_mut();
    match operation {
        Subscription::Instant => {
            monotonic_clock::Host::subscribe_instant(&mut data.clocks(), *value)
        }
        Subscription::Duration => {
            monotonic_clock::Host::subscribe_duration(&mut data.clocks(), *value)
        }
    }
}
