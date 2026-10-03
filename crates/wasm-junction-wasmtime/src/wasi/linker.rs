use wasmtime::component::Linker;
use wasmtime_wasi::cli::{WasiCli, WasiCliView};
use wasmtime_wasi::p2::bindings::cli;

use crate::engine::StoreData;

pub(crate) fn add_ungated_interfaces(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    cli::exit::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_input::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_output::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_stdin::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_stdout::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_stderr::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    Ok(())
}
