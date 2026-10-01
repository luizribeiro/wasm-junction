use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi::WasiView;
use wasmtime_wasi::cli::{WasiCli, WasiCliView};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView};
use wasmtime_wasi::p2::bindings::{cli, filesystem, io, random, sockets};
use wasmtime_wasi::random::{WasiRandom, WasiRandomView};
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsView};

use crate::engine::StoreData;

struct HasIo;

impl HasData for HasIo {
    type Data<'a> = &'a mut ResourceTable;
}

pub(crate) fn add_ungated_interfaces(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    io::error::add_to_linker::<StoreData, HasIo>(linker, |state| state.ctx().table)?;
    io::poll::add_to_linker::<StoreData, HasIo>(linker, |state| state.ctx().table)?;
    io::streams::add_to_linker::<StoreData, HasIo>(linker, |state| state.ctx().table)?;
    filesystem::preopens::add_to_linker::<StoreData, WasiFilesystem>(
        linker,
        WasiFilesystemView::filesystem,
    )?;
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
    sockets::tcp_create_socket::add_to_linker::<StoreData, WasiSockets>(
        linker,
        WasiSocketsView::sockets,
    )?;
    sockets::instance_network::add_to_linker::<StoreData, WasiSockets>(
        linker,
        WasiSocketsView::sockets,
    )?;
    let options = wasmtime_wasi::p2::bindings::LinkOptions::default();
    sockets::network::add_to_linker::<StoreData, WasiSockets>(
        linker,
        &(&options).into(),
        WasiSocketsView::sockets,
    )?;
    filesystem::types::add_to_linker::<StoreData, WasiFilesystem>(
        linker,
        WasiFilesystemView::filesystem,
    )?;
    sockets::tcp::add_to_linker::<StoreData, WasiSockets>(linker, WasiSocketsView::sockets)?;
    sockets::udp::add_to_linker::<StoreData, WasiSockets>(linker, WasiSocketsView::sockets)?;
    sockets::udp_create_socket::add_to_linker::<StoreData, WasiSockets>(
        linker,
        WasiSocketsView::sockets,
    )?;
    sockets::ip_name_lookup::add_to_linker::<StoreData, WasiSockets>(
        linker,
        WasiSocketsView::sockets,
    )
}
