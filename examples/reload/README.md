# Reload

This example replaces a running greeter component without interrupting a call that already began.
Both guest generations implement the same interface, but include a visible `v1` or `v2` marker in
their answers.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-reload
```

## In the browser

Run the dedicated-Worker browser test in Chromium, Firefox, and WebKit:

```sh
nix develop -c scripts/browser-test all -- -p wasm-junction-example-reload --lib
```

The native entry point and the browser test call the same `run` function. In the browser, both
embedded generations are transpiled to JavaScript on the Worker's thread, including v2 during
the reload itself.

The executable tells one short lifecycle story:

1. It loads `greeter` v1 and calls it through a typed handle.
2. It starts another call, which waits on a host import controlled by the example.
3. While that call is suspended, it reloads `greeter` with v2.
4. A new call through the same handle receives v2's answer.
5. Releasing the suspended call lets it finish with v1's answer.

A call selects and retains its component generation when it starts. Reload swaps the generation
used by later calls, while the earlier call keeps its old compiled component alive until it ends.
A typed handle names the component rather than a particular generation, so existing handles follow
the replacement automatically.

Tracing middleware prints every call boundary plus the load and reload events. The output contains
no timing data, and an integration test checks the complete transcript.

The files are intentionally self-contained:

- `wit/reload.wit` defines the gate import and greeter export.
- `guest/` is an isolated Rust workspace containing the v1 and v2 guests.
- `build.rs` builds both guests for `wasm32-unknown-unknown` and componentizes them without WASI.
- `src/gate.rs` implements the host-controlled suspension point.
- `src/trace.rs` contains the example's tracing middleware.
- `src/lib.rs` loads, calls, reloads, and releases the two generations.
- `src/main.rs` uses Tokio to run that target-neutral workflow and print its output.
- `tests/` runs the executable and checks its complete output.
