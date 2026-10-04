#[allow(
    clippy::wildcard_imports,
    reason = "CLI gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime::AsContextMut;
use wasmtime::component::{Access, Accessor, FutureReader, StreamReader};
use wasmtime_wasi::cli::{WasiCli, WasiCliView};
use wasmtime_wasi::p3::bindings::cli::{
    environment, exit, stderr, stdin, stdout, terminal_input, terminal_output, terminal_stderr,
    terminal_stdin, terminal_stdout, types::ErrorCode,
};
use wasmtime_wasi::p3::cli::{TerminalInput, TerminalOutput};

const TERMINAL_INPUT_INTERFACE: &str = "wasi:cli/terminal-input@0.3.0";
const TERMINAL_OUTPUT_INTERFACE: &str = "wasi:cli/terminal-output@0.3.0";
const TERMINAL_INPUT_NAME: &str = "terminal-input";
const TERMINAL_OUTPUT_NAME: &str = "terminal-output";

impl WitResource for TerminalInput {
    const INTERFACE: &'static str = TERMINAL_INPUT_INTERFACE;
    const NAME: &'static str = TERMINAL_INPUT_NAME;
}

impl WitResource for TerminalOutput {
    const INTERFACE: &'static str = TERMINAL_OUTPUT_INTERFACE;
    const NAME: &'static str = TERMINAL_OUTPUT_NAME;
}

type TransferResult = Result<(), ErrorCode>;
type WriteOutput = for<'a> fn(
    Access<'a, StoreData, WasiCli>,
    StreamReader<u8>,
) -> wasmtime::Result<FutureReader<TransferResult>>;

fn refused_future(
    store: &mut StoreContextMut<'_, StoreData>,
) -> wasmtime::Result<FutureReader<TransferResult>> {
    FutureReader::new(store.as_context_mut(), async {
        Ok::<TransferResult, wasmtime::Error>(Err(ErrorCode::Io))
    })
}

fn write_output_real(
    accessor: &Accessor<StoreData>,
    args: Vals,
    write: WriteOutput,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        let [stream] = <[Val; 1]>::try_from(args).map_err(|_| shape("output stream"))?;
        accessor.with(|mut access| {
            let mut store = access.as_context_mut();
            let stream = lower_stream_handoff_plain(&mut store, stream)
                .map_err(|error| CallError::trap(error.to_string()))?;
            let cli = Access::<StoreData, WasiCli>::new(store.as_context_mut(), WasiCliView::cli);
            let future = write(cli, stream).map_err(|error| CallError::trap(error.to_string()))?;
            lift_future_plain(&mut store, future)
                .map(|future| vec![future])
                .map_err(|error| CallError::trap(error.to_string()))
        })
    })
}

fn write_stdout_real(
    accessor: &Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    write_output_real(accessor, args, stdout::HostWithStore::write_via_stream)
}

fn write_stderr_real(
    accessor: &Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    write_output_real(accessor, args, stderr::HostWithStore::write_via_stream)
}

fn add_output(
    linker: &mut Linker<StoreData>,
    interface: &'static str,
    real: RealConcurrent,
) -> wasmtime::Result<()> {
    linker.instance(interface)?.func_wrap(
        "write-via-stream",
        move |mut store, (stream,): (StreamReader<u8>,)| {
            let stream = lift_stream_with_direction_plain(
                &mut store,
                stream,
                ChannelDirection::GuestToHost,
            )?;
            let future = super::deferred::spawn(
                &mut store,
                interface,
                "write-via-stream",
                vec![stream],
                real,
                ErrorCode::Io,
            )?;
            Ok((future,))
        },
    )?;
    Ok(())
}

fn add_stdin(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    const INTERFACE: &str = "wasi:cli/stdin@0.3.0";
    linker
        .instance(INTERFACE)?
        .func_wrap_async("read-via-stream", |mut store, (): ()| {
            Box::new(async move {
                let real: Real = |mut store, _args| {
                    Box::pin(async move {
                        let access = Access::<StoreData, WasiCli>::new(
                            store.as_context_mut(),
                            WasiCliView::cli,
                        );
                        let (stream, future) = stdin::HostWithStore::read_via_stream(access)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        Ok(vec![Val::Tuple(vec![
                            lift_stream_with_direction_plain(
                                &mut store,
                                stream,
                                ChannelDirection::HostToGuest,
                            )
                            .map_err(|error| CallError::trap(error.to_string()))?,
                            lift_future_plain(&mut store, future)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                        ])])
                    })
                };
                let outcome =
                    trampoline::gate(&mut store, INTERFACE, "read-via-stream", Vec::new(), real)
                        .await;
                let values = match outcome {
                    Ok(values) => values,
                    Err(error) if error.kind() != CallErrorKind::Refused => {
                        return Err(wasmtime::Error::new(error));
                    }
                    Err(_) => {
                        let (writer, stream) = wasm_junction_core::OutputStream::channel();
                        drop(writer);
                        let stream = crate::streams::lower_stream(
                            wasm_junction_core::StreamHandle::from(stream),
                            store.as_context_mut(),
                        )?;
                        return Ok(((
                            StreamReader::try_from_stream_any(stream)?,
                            refused_future(&mut store)?,
                        ),));
                    }
                };
                let [Val::Tuple(values)] = values.as_slice() else {
                    return Err(wasmtime::Error::new(shape("stream and future")));
                };
                let [stream, future] = values.as_slice() else {
                    return Err(wasmtime::Error::new(shape("stream and future")));
                };
                Ok(((
                    lower_stream_handoff_plain(&mut store, stream.clone())?,
                    lower_future_plain(&mut store, future.clone())?,
                ),))
            })
        })?;
    Ok(())
}

fn drop_terminal_input(
    store: &mut StoreData,
    terminal: Resource<TerminalInput>,
) -> wasmtime::Result<()> {
    terminal_input::HostTerminalInput::drop(&mut views::cli(store), terminal)
}

fn drop_terminal_output(
    store: &mut StoreData,
    terminal: Resource<TerminalOutput>,
) -> wasmtime::Result<()> {
    terminal_output::HostTerminalOutput::drop(&mut views::cli(store), terminal)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance("wasi:cli/types@0.3.0")?;
    gate!(linker, "wasi:cli/environment@0.3.0", "get-environment", cli,
        environment::Host::get_environment, plain, () -> Vec<(String, String)>);
    gate!(linker, "wasi:cli/environment@0.3.0", "get-arguments", cli,
        environment::Host::get_arguments, plain, () -> Vec<String>);
    gate!(linker, "wasi:cli/environment@0.3.0", "get-initial-cwd", cli,
        environment::Host::get_initial_cwd, plain, () -> Option<String>);
    gate!(linker, "wasi:cli/exit@0.3.0", "exit", cli,
        exit::Host::exit, plain, (status: Result<(), ()>) -> ());
    gate!(linker, "wasi:cli/exit@0.3.0", "exit-with-code", cli,
        exit::Host::exit_with_code, plain, (status_code: u8) -> ());
    add_stdin(linker)?;
    add_output(linker, "wasi:cli/stdout@0.3.0", write_stdout_real)?;
    add_output(linker, "wasi:cli/stderr@0.3.0", write_stderr_real)?;
    gate_drop!(
        linker,
        TERMINAL_INPUT_INTERFACE,
        TERMINAL_INPUT_NAME,
        "[drop]terminal-input",
        TerminalInput,
        store,
        None,
        drop_terminal_input
    );
    gate_drop!(
        linker,
        TERMINAL_OUTPUT_INTERFACE,
        TERMINAL_OUTPUT_NAME,
        "[drop]terminal-output",
        TerminalOutput,
        store,
        None,
        drop_terminal_output
    );
    gate!(linker, "wasi:cli/terminal-stdin@0.3.0", "get-terminal-stdin", cli,
        terminal_stdin::Host::get_terminal_stdin,
        resource, () -> Option<Resource<TerminalInput>>);
    gate!(linker, "wasi:cli/terminal-stdout@0.3.0", "get-terminal-stdout", cli,
        terminal_stdout::Host::get_terminal_stdout,
        resource, () -> Option<Resource<TerminalOutput>>);
    gate!(linker, "wasi:cli/terminal-stderr@0.3.0", "get-terminal-stderr", cli,
        terminal_stderr::Host::get_terminal_stderr,
        resource, () -> Option<Resource<TerminalOutput>>);
    Ok(())
}
