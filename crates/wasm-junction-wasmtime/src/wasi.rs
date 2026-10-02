use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction_core::{BoxFuture, CallError, ImportTarget, InvocationContext, Val, Vals};
use wasmtime::component::{Linker, ResourceTable};
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi::cli::WasiCliView;
use wasmtime_wasi::p2::bindings::cli;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::GATED_WASI_INTERFACES;
use crate::engine::StoreData;

mod clocks;
mod gates;
mod linker;
mod trampoline;

pub(crate) use linker::add_ungated_interfaces;

const INTERFACE: &str = GATED_WASI_INTERFACES[0];

pub(crate) struct WasiState {
    pub(crate) context: WasiCtx,
    pub(crate) table: ResourceTable,
}

impl WasiState {
    pub(crate) fn new(context: WasiCtx) -> Self {
        Self {
            context,
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for WasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.context,
            table: &mut self.table,
        }
    }
}

#[derive(Clone, Copy)]
enum EnvironmentOperation {
    Arguments,
}

struct EnvironmentTarget(Arc<Mutex<WasiState>>, EnvironmentOperation);

impl ImportTarget for EnvironmentTarget {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        let state = self.0.clone();
        let operation = self.1;
        Box::pin(async move {
            if !args.is_empty() {
                return Err(CallError::trap("WASI environment call takes no arguments"));
            }
            let mut state = lock(&state);
            match operation {
                EnvironmentOperation::Arguments => {
                    cli::environment::Host::get_arguments(&mut state.cli())
                        .map(|values| Val::List(values.into_iter().map(Val::from).collect()))
                }
            }
            .map(|value| vec![value])
            .map_err(|error| CallError::trap(error.to_string()))
        })
    }
}

pub(crate) fn add_gates(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gates::add_environment(linker)?;
    add_environment_gate(linker)?;
    clocks::add_wall_clock_gate(linker)
}

fn add_environment_gate(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    let mut instance = linker.instance(INTERFACE)?;
    instance.func_wrap_async("get-arguments", |mut store, (): ()| {
        Box::new(async move {
            let values =
                call_environment(&mut store, "get-arguments", EnvironmentOperation::Arguments)
                    .await?;
            Ok((decode_arguments(&values)?,))
        })
    })?;
    instance.func_wrap_async("initial-cwd", |mut store, (): ()| {
        Box::new(async move {
            let values = trampoline::gate(
                &mut store,
                INTERFACE,
                "initial-cwd",
                Vec::new(),
                real_initial_cwd,
            )
            .await
            .map_err(wasmtime::Error::new)?;
            Ok((decode_initial_cwd(&values)?,))
        })
    })
}

fn real_initial_cwd(
    mut store: StoreContextMut<'_, StoreData>,
    args: Vals,
) -> BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        if !args.is_empty() {
            return Err(CallError::trap("WASI initial-cwd takes no arguments"));
        }
        cli::environment::Host::initial_cwd(&mut store.data_mut().cli())
            .map(|value| vec![Val::Option(value.map(|value| Box::new(Val::from(value))))])
            .map_err(|error| CallError::trap(error.to_string()))
    })
}

async fn call_environment(
    store: &mut StoreContextMut<'_, StoreData>,
    function: &'static str,
    operation: EnvironmentOperation,
) -> wasmtime::Result<Vals> {
    let target = {
        let mut context = store.as_context_mut();
        let data = context.data_mut();
        Arc::new(EnvironmentTarget(data.gated_wasi.clone(), operation))
    };
    dispatch(store, INTERFACE, function, Vec::new(), target).await
}

pub(super) async fn dispatch(
    store: &mut StoreContextMut<'_, StoreData>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
    target: Arc<dyn ImportTarget>,
) -> wasmtime::Result<Vals> {
    let (imports, context, component) = {
        let mut store = store.as_context_mut();
        let data = store.data_mut();
        (
            data.imports.clone(),
            data.context.clone(),
            data.component.clone(),
        )
    };
    imports
        .call_engine(
            context,
            component,
            Arc::from(interface),
            Arc::from(function),
            args,
            target,
        )
        .await
        .map_err(wasmtime::Error::new)
}

fn decode_arguments(values: &[Val]) -> wasmtime::Result<Vec<String>> {
    let [Val::List(values)] = values else {
        return Err(wasmtime::Error::msg(
            "get-arguments returned the wrong shape",
        ));
    };
    values
        .iter()
        .map(|value| match value {
            Val::String(value) => Ok(value.clone()),
            _ => Err(wasmtime::Error::msg("argument is not a string")),
        })
        .collect()
}

fn decode_initial_cwd(values: &[Val]) -> wasmtime::Result<Option<String>> {
    match values {
        [Val::Option(Some(value))] => match value.as_ref() {
            Val::String(value) => Ok(Some(value.clone())),
            _ => Err(wasmtime::Error::msg("initial-cwd is not a string")),
        },
        [Val::Option(None)] => Ok(None),
        _ => Err(wasmtime::Error::msg("initial-cwd returned the wrong shape")),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}
