use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::oneshot;
use wasmtime::AsContextMut;
use wasmtime::component::{
    Accessor, ComponentType, FutureConsumer, FutureReader, Lift, Lower, Source,
};

use super::{
    CallErrorKind, RealConcurrent, StoreContextMut, StoreData, Val, Vals, shape, trampoline,
};

type Completion<E> = wasmtime::Result<Result<(), E>>;

struct CompletionConsumer<E>(Option<oneshot::Sender<Completion<E>>>);

impl<E: ComponentType + Lift + Send + 'static> FutureConsumer<StoreData> for CompletionConsumer<E> {
    type Item = Result<(), E>;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut store: StoreContextMut<'_, StoreData>,
        mut source: Source<'_, Self::Item>,
        finish: bool,
    ) -> Poll<wasmtime::Result<()>> {
        let mut value = None;
        source.read(store.as_context_mut(), &mut value)?;
        if let Some(value) = value {
            if let Some(sender) = self.get_mut().0.take() {
                let _ = sender.send(Ok(value));
            }
            return Poll::Ready(Ok(()));
        }
        if finish {
            if let Some(sender) = self.get_mut().0.take() {
                let _ = sender.send(Err(wasmtime::Error::msg("completion was canceled")));
            }
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }
}

async fn run_gate<E: ComponentType + Lift + Send + 'static>(
    accessor: &Accessor<StoreData>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
    real: RealConcurrent,
    denied: E,
    completion: oneshot::Sender<Completion<E>>,
) -> wasmtime::Result<()> {
    let outcome = trampoline::gate_concurrent(accessor, interface, function, args, real).await;
    let values = match outcome {
        Ok(values) => values,
        Err(error) if error.kind() == CallErrorKind::Refused => {
            let _ = completion.send(Ok(Err(denied)));
            return Ok(());
        }
        Err(error) => {
            let _ = completion.send(Err(wasmtime::Error::new(error)));
            return Ok(());
        }
    };
    let [future] = <[Val; 1]>::try_from(values).map_err(|_| shape("future"))?;
    let future = accessor
        .with(|mut access| super::lower_future_plain(&mut access.as_context_mut(), future))?;
    accessor.with(|mut access| {
        future.pipe(
            access.as_context_mut(),
            CompletionConsumer(Some(completion)),
        )
    })
}

pub(super) fn spawn<E: ComponentType + Lift + Lower + Send + Sync + 'static>(
    store: &mut StoreContextMut<'_, StoreData>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
    real: RealConcurrent,
    denied: E,
) -> wasmtime::Result<FutureReader<Result<(), E>>> {
    let (completion, receiver) = oneshot::channel();
    let future = FutureReader::new(store.as_context_mut(), async move {
        receiver
            .await
            .map_err(|_| wasmtime::Error::msg("gate ended without a result"))?
    })?;
    store.as_context_mut().spawn(async move |accessor| {
        run_gate(
            accessor, interface, function, args, real, denied, completion,
        )
        .await
    })?;
    Ok(future)
}
