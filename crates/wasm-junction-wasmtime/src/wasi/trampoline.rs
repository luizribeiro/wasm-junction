use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use wasm_junction_core::{BoxFuture, CallError, ImportTarget, InvocationContext, Vals};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;

type Reply = oneshot::Sender<Result<Vals, CallError>>;

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
    let (requests, mut pending) = mpsc::unbounded_channel();
    // Wasmtime's wrapper future owns this borrowed store, while middleware futures must be
    // `'static`; the target asks this stack frame to perform each real call on its behalf.
    let mut chain = imports.call_engine(
        context,
        component,
        Arc::from(interface),
        Arc::from(function),
        args,
        Arc::new(StoreAccess(requests)),
    );
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
