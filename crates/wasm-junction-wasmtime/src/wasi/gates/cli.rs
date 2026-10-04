#[allow(
    clippy::wildcard_imports,
    reason = "CLI gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::p3::bindings::cli::{
    environment, exit, terminal_input, terminal_output, terminal_stderr, terminal_stdin,
    terminal_stdout,
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
