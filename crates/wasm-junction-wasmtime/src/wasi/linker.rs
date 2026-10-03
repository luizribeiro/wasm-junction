use wasmtime::component::Linker;
use wasmtime_wasi::cli::{WasiCli, WasiCliView};
use wasmtime_wasi::p2::bindings::{cli, random};
use wasmtime_wasi::random::{WasiRandom, WasiRandomView};

use crate::engine::StoreData;

pub(crate) fn add_ungated_interfaces(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    random::random::add_to_linker::<StoreData, WasiRandom>(linker, WasiRandomView::random)?;
    random::insecure::add_to_linker::<StoreData, WasiRandom>(linker, WasiRandomView::random)?;
    random::insecure_seed::add_to_linker::<StoreData, WasiRandom>(linker, WasiRandomView::random)?;
    cli::exit::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::stdin::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::stdout::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::stderr::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_input::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_output::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_stdin::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_stdout::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    cli::terminal_stderr::add_to_linker::<StoreData, WasiCli>(linker, WasiCliView::cli)?;
    Ok(())
}
