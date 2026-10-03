use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use wasm_junction_core::{
    BoxFuture, CallError, ImportDispatcher, ImportTarget, InvocationContext, Vals,
};
#[cfg(feature = "wasi-p3")]
use wasmtime::component::Accessor;
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;

type Reply = oneshot::Sender<Result<Vals, CallError>>;
type Pending = mpsc::UnboundedReceiver<(Vals, Reply)>;

struct StoreAccess(mpsc::UnboundedSender<(Vals, Reply)>);

impl ImportTarget for StoreAccess {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        let requests = self.0.clone();
        Box::pin(async move {
            let (reply, response) = oneshot::channel();
            requests
                .send((args, reply))
                .map_err(|_| CallError::unavailable("the WASI call has already returned"))?;
            response
                .await
                .map_err(|_| CallError::unavailable("the WASI call was abandoned"))?
        })
    }
}

pub(super) type Real =
    for<'a> fn(StoreContextMut<'a, StoreData>, Vals) -> BoxFuture<'a, Result<Vals, CallError>>;

pub(super) async fn gate(
    store: &mut StoreContextMut<'_, StoreData>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
    real: Real,
) -> Result<Vals, CallError> {
    let (imports, context, component) = {
        let data = store.data();
        (
            data.imports.clone(),
            data.context.clone(),
            data.component.clone(),
        )
    };
    let (mut chain, mut pending) = start(imports, context, component, interface, function, args);
    loop {
        tokio::select! {
            result = &mut chain => return result,
            Some((args, reply)) = pending.recv() => {
                let mut real_call = real(store.as_context_mut(), args);
                // Middleware may refuse or otherwise finish without ever invoking `next`.
                let outcome = tokio::select! {
                    outcome = &mut real_call => outcome,
                    result = &mut chain => return result,
                };
                drop(real_call);
                let _ = reply.send(outcome);
            }
        }
    }
}

fn start(
    imports: Arc<dyn ImportDispatcher>,
    context: InvocationContext,
    component: Arc<str>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
) -> (BoxFuture<'static, Result<Vals, CallError>>, Pending) {
    let (requests, pending) = mpsc::unbounded_channel();
    // Wasmtime's wrapper future owns this borrowed store, while middleware futures must be
    // `'static`; the target asks this stack frame to perform each real call on its behalf.
    let chain: BoxFuture<'static, _> = Box::pin(async move {
        imports
            .call_engine(
                context,
                component,
                Arc::from(interface),
                Arc::from(function),
                args,
                Arc::new(StoreAccess(requests)),
            )
            .await
    });
    (chain, pending)
}

#[cfg(feature = "wasi-p3")]
pub(super) type RealConcurrent =
    for<'a> fn(&'a Accessor<StoreData>, Vals) -> BoxFuture<'a, Result<Vals, CallError>>;

#[cfg(feature = "wasi-p3")]
pub(super) async fn gate_concurrent(
    accessor: &Accessor<StoreData>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
    real: RealConcurrent,
) -> Result<Vals, CallError> {
    let (imports, context, component) = accessor.with(|mut access| {
        let data = access.get();
        (
            data.imports.clone(),
            data.context.clone(),
            data.component.clone(),
        )
    });
    let (mut chain, mut pending) = start(imports, context, component, interface, function, args);
    loop {
        tokio::select! {
            result = &mut chain => return result,
            Some((args, reply)) = pending.recv() => {
                let mut real_call = real(accessor, args);
                let outcome = tokio::select! {
                    outcome = &mut real_call => outcome,
                    result = &mut chain => return result,
                };
                drop(real_call);
                let _ = reply.send(outcome);
            }
        }
    }
}
