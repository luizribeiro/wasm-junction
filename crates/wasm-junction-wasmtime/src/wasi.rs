use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction_core::{BoxFuture, CallError, ImportTarget, InvocationContext, Val, Vals};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi::cli::WasiCliView;
use wasmtime_wasi::p2::bindings::cli::environment;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::GATED_WASI_INTERFACES;
use crate::engine::StoreData;

mod clocks;
mod linker;

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

struct Gate<'a>(&'a mut StoreData);
struct GateData(PhantomData<StoreData>);

impl HasData for GateData {
    type Data<'a> = Gate<'a>;
}

fn project(state: &mut StoreData) -> Gate<'_> {
    Gate(state)
}

#[derive(Clone, Copy)]
enum EnvironmentOperation {
    Variables,
    Arguments,
    InitialCwd,
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
                EnvironmentOperation::Variables => {
                    environment::Host::get_environment(&mut state.cli()).map(|entries| {
                        Val::List(
                            entries
                                .into_iter()
                                .map(|(name, value)| Val::Tuple(vec![name.into(), value.into()]))
                                .collect(),
                        )
                    })
                }
                EnvironmentOperation::Arguments => {
                    environment::Host::get_arguments(&mut state.cli())
                        .map(|values| Val::List(values.into_iter().map(Val::from).collect()))
                }
                EnvironmentOperation::InitialCwd => {
                    environment::Host::initial_cwd(&mut state.cli())
                        .map(|value| Val::Option(value.map(|value| Box::new(Val::from(value)))))
                }
            }
            .map(|value| vec![value])
            .map_err(|error| CallError::trap(error.to_string()))
        })
    }
}

impl environment::Host for Gate<'_> {
    fn get_environment(&mut self) -> wasmtime::Result<Vec<(String, String)>> {
        decode_environment(&self.call("get-environment", EnvironmentOperation::Variables)?)
    }

    fn get_arguments(&mut self) -> wasmtime::Result<Vec<String>> {
        let values = self.call("get-arguments", EnvironmentOperation::Arguments)?;
        let [Val::List(values)] = values.as_slice() else {
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

    fn initial_cwd(&mut self) -> wasmtime::Result<Option<String>> {
        let values = self.call("initial-cwd", EnvironmentOperation::InitialCwd)?;
        match values.as_slice() {
            [Val::Option(Some(value))] => match value.as_ref() {
                Val::String(value) => Ok(Some(value.clone())),
                _ => Err(wasmtime::Error::msg("initial-cwd is not a string")),
            },
            [Val::Option(None)] => Ok(None),
            _ => Err(wasmtime::Error::msg("initial-cwd returned the wrong shape")),
        }
    }
}

impl Gate<'_> {
    fn dispatch(
        &mut self,
        interface: impl Into<Arc<str>>,
        function: impl Into<Arc<str>>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> wasmtime::Result<Vals> {
        futures::executor::block_on(self.0.imports.call_engine(
            self.0.context.clone(),
            self.0.component.clone(),
            interface.into(),
            function.into(),
            args,
            target,
        ))
        .map_err(wasmtime::Error::new)
    }

    fn call(
        &mut self,
        function: &'static str,
        operation: EnvironmentOperation,
    ) -> wasmtime::Result<Vals> {
        self.dispatch(
            INTERFACE,
            function,
            Vec::new(),
            Arc::new(EnvironmentTarget(self.0.gated_wasi.clone(), operation)),
        )
    }
}

pub(crate) fn add_gates(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.allow_shadowing(true);
    let result = environment::add_to_linker::<StoreData, GateData>(linker, project)
        .and_then(|()| clocks::add_wall_clock_gate(linker));
    linker.allow_shadowing(false);
    result
}

fn decode_environment(values: &[Val]) -> wasmtime::Result<Vec<(String, String)>> {
    let [Val::List(entries)] = values else {
        return Err(wasmtime::Error::msg(
            "get-environment returned the wrong shape",
        ));
    };
    entries
        .iter()
        .map(|entry| match entry {
            Val::Tuple(fields) => match fields.as_slice() {
                [Val::String(name), Val::String(value)] => Ok((name.clone(), value.clone())),
                _ => Err(wasmtime::Error::msg(
                    "environment tuple has the wrong shape",
                )),
            },
            _ => Err(wasmtime::Error::msg("environment entry is not a tuple")),
        })
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}
