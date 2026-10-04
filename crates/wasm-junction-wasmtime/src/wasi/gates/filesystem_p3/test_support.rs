use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use tokio::sync::oneshot;
use wasm_junction_core::{
    BoxFuture, CallError, ImportDispatcher, ImportTarget, InvocationContext, InvocationId,
    Resource as JunctionResource, Vals,
};
use wasmtime::component::{FutureConsumer, Source};
use wasmtime::{AsContextMut, Config, Engine, Store, StoreContextMut};
use wasmtime_wasi::p3::bindings::filesystem::preopens;
use wasmtime_wasi::p3::bindings::filesystem::types::ErrorCode;
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

use super::{StoreData, lower_future_plain};
use crate::wasi::WasiState;
use crate::wasi::gates::views;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

pub(super) struct TestDirectory(PathBuf);

impl TestDirectory {
    pub(super) fn new(name: &str) -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("wasm-junction-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct TestDispatcher {
    refused: Option<&'static str>,
}

impl TestDispatcher {
    pub(super) const fn passing() -> Self {
        Self { refused: None }
    }

    pub(super) const fn refusing(function: &'static str) -> Self {
        Self {
            refused: Some(function),
        }
    }
}

impl ImportDispatcher for TestDispatcher {
    fn call(
        &self,
        _context: InvocationContext,
        _caller: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        _args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async { Err(CallError::trap("unexpected imported call")) })
    }

    fn call_engine(
        &self,
        context: InvocationContext,
        _caller: Arc<str>,
        _interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        if self.refused == Some(function.as_ref()) {
            Box::pin(async { Err(CallError::refused("denied by test middleware")) })
        } else {
            target.call(context, args)
        }
    }

    fn drop_resource(
        &self,
        _context: InvocationContext,
        _caller: Arc<str>,
        _resource: JunctionResource,
    ) -> BoxFuture<'_, Result<(), CallError>> {
        Box::pin(async { Ok(()) })
    }
}

pub(super) fn store(
    preopens: &[(&Path, &str, FsPerms)],
    dispatcher: TestDispatcher,
) -> Store<StoreData> {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let mut builder = WasiCtxBuilder::new();
    for (path, guest_path, perms) in preopens {
        builder.preopened_dir(path, *guest_path, *perms).unwrap();
    }
    let invocation = InvocationId::__from_counter(1);
    let context = InvocationContext::default().with_invocation_id(invocation);
    Store::new(
        &engine,
        StoreData::for_wasi_test(
            Arc::new(dispatcher),
            context,
            WasiState::new(builder.build()),
        ),
    )
}

pub(super) fn preopens(accessor: &wasmtime::component::Accessor<StoreData>) -> Vec<(u32, String)> {
    accessor
        .with(|mut access| -> wasmtime::Result<_> {
            let store = access.get();
            let directories = preopens::Host::get_directories(&mut views::filesystem(store))?;
            for (descriptor, path) in &directories {
                store.set_descriptor_preopen(descriptor.rep(), path.clone());
            }
            Ok(directories
                .into_iter()
                .map(|(descriptor, path)| (descriptor.rep(), path))
                .collect())
        })
        .unwrap()
}

struct CompletionConsumer(Option<oneshot::Sender<Result<(), ErrorCode>>>);

impl FutureConsumer<StoreData> for CompletionConsumer {
    type Item = Result<(), ErrorCode>;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut store: StoreContextMut<'_, StoreData>,
        mut source: Source<'_, Self::Item>,
        finish: bool,
    ) -> Poll<wasmtime::Result<()>> {
        let mut result = None;
        source.read(store.as_context_mut(), &mut result)?;
        if let Some(result) = result {
            if let Some(sender) = self.get_mut().0.take() {
                let _ = sender.send(result);
            }
            Poll::Ready(Ok(()))
        } else if finish {
            Poll::Ready(Err(wasmtime::Error::msg(
                "completion ended without a value",
            )))
        } else {
            Poll::Pending
        }
    }
}

pub(super) async fn completion(
    accessor: &wasmtime::component::Accessor<StoreData>,
    value: wasm_junction_core::Val,
) -> Result<(), ErrorCode> {
    let future = accessor
        .with(|mut access| lower_future_plain(&mut access.as_context_mut(), value))
        .unwrap();
    let (sender, receiver) = oneshot::channel();
    accessor
        .with(|mut access| future.pipe(access.as_context_mut(), CompletionConsumer(Some(sender))))
        .unwrap();
    receiver.await.unwrap()
}
