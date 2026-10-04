use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use wasm_junction_core::{
    BoxFuture, CallError, ImportDispatcher, ImportTarget, InvocationContext, InvocationId,
    Resource as JunctionResource, Vals,
};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p3::bindings::filesystem::preopens;
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

use super::StoreData;
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

pub(super) struct TestDispatcher;

impl TestDispatcher {
    pub(super) const fn passing() -> Self {
        Self
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
        _function: Arc<str>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        target.call(context, args)
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
