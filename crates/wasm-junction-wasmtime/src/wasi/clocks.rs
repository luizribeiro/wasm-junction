use std::sync::Arc;

use wasm_junction_core::{BoxFuture, CallError, ImportTarget, InvocationContext, Val, Vals};
use wasmtime::component::{Linker, Resource};
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi::clocks::WasiClocksView;
use wasmtime_wasi::p2::DynPollable;
use wasmtime_wasi::p2::bindings::clocks::monotonic_clock;

use super::{StoreData, dispatch};
use crate::GATED_WASI_INTERFACES;

const MONOTONIC_INTERFACE: &str = GATED_WASI_INTERFACES[1];
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

pub(super) fn add_subscriptions(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    let mut instance = linker.instance(MONOTONIC_INTERFACE)?;
    instance.func_wrap_async("subscribe-instant", |mut store, (value,): (u64,)| {
        Box::new(async move { Ok((subscribe(&mut store, Subscription::Instant, value).await?,)) })
    })?;
    instance.func_wrap_async("subscribe-duration", |mut store, (value,): (u64,)| {
        Box::new(async move { Ok((subscribe(&mut store, Subscription::Duration, value).await?,)) })
    })
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
