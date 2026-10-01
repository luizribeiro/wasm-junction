use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction_core::{BoxFuture, CallError, ImportTarget, InvocationContext, Val, Vals};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi::cli::WasiCliView;
use wasmtime_wasi::p2::bindings::cli::environment;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::engine::StoreData;

const INTERFACE: &str = "wasi:cli/environment@0.2.12";

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

struct EnvironmentTarget(Arc<Mutex<WasiState>>);

impl ImportTarget for EnvironmentTarget {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        let state = self.0.clone();
        Box::pin(async move {
            if !args.is_empty() {
                return Err(CallError::trap("get-environment takes no arguments"));
            }
            let entries = environment::Host::get_environment(&mut lock(&state).cli())
                .map_err(|error| CallError::trap(error.to_string()))?;
            Ok(vec![Val::List(
                entries
                    .into_iter()
                    .map(|(name, value)| Val::Tuple(vec![name.into(), value.into()]))
                    .collect(),
            )])
        })
    }
}

impl environment::Host for Gate<'_> {
    fn get_environment(&mut self) -> wasmtime::Result<Vec<(String, String)>> {
        let values = futures::executor::block_on(self.0.imports.call_engine(
            self.0.context.clone(),
            self.0.component.clone(),
            Arc::from(INTERFACE),
            Arc::from("get-environment"),
            Vec::new(),
            Arc::new(EnvironmentTarget(self.0.gated_wasi.clone())),
        ))
        .map_err(wasmtime::Error::new)?;
        decode_environment(&values)
    }

    fn get_arguments(&mut self) -> wasmtime::Result<Vec<String>> {
        environment::Host::get_arguments(&mut self.0.cli())
    }

    fn initial_cwd(&mut self) -> wasmtime::Result<Option<String>> {
        environment::Host::initial_cwd(&mut self.0.cli())
    }
}

pub(crate) fn add_environment_gate(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.allow_shadowing(true);
    let result = environment::add_to_linker::<StoreData, GateData>(linker, project);
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
