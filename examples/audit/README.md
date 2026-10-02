# Audit

This example combines a host resource, a guest-produced byte stream, and per-call data in one
small workflow.

First, the host provides a `session` resource. The guest opens one for Ada, calls its `user`
method, and drops it after the audit is complete. The tracing middleware prints the constructor,
method, and resource-drop event.

Next, the guest creates a `stream<u8>` and writes three log lines while the host's asynchronous
`audit` function reads them. Channel-open and channel-close events show the guest-to-host stream
crossing the engine boundary.

Finally, the host calls the guest with `.with(RequestId(42))`. That per-call value follows the
nested calls into the audit provider, which reads it from `cx.extensions()` and prefixes every
stored line. The final three lines print the stored, tagged audit entries.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-audit
```

## In the browser

Run the dedicated-Worker browser test in Chromium, Firefox, and WebKit:

```sh
nix develop -c scripts/browser-test all -- -p wasm-junction-example-audit --lib
```

The native entry point and the browser test call the same `run` function. In a browser, loading
the embedded component also transpiles it to JavaScript on the Worker's thread.

Everything used by the example lives here: `wit/` defines the interfaces and world, `guest/` is
the isolated component workspace, `build.rs` builds and componentizes it without WASI,
`src/bindings.rs` generates host bindings, `src/trace.rs` contains the middleware, `src/lib.rs`
runs the target-neutral workflow, and `src/main.rs` uses Tokio to print its output. `tests/`
checks the executable's complete output.
