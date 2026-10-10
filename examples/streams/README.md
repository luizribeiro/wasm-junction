# Streams

This example shows middleware changing both byte and typed streams in a support-ticket workflow.
The guest sends a transcript to the host in two chunks, with `ada@example.com` split between
them. The middleware uses `map_chunks_with_flush` and keeps partial matches between calls, so the
host stores the transcript with the address redacted. Its flush callback emits a partial candidate
at the end because, without the remaining characters, it cannot be the email being redacted.

The host then returns a `stream<string>` of public and private tickets. The same middleware uses
`filter_items` to remove private tickets before the guest reads them, and the guest summarises only
the two visible subjects. Strings are used because Wasmtime cannot carry named record types in
application streams yet.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-streams
```

## In the browser

Run the dedicated-Worker browser test in Chromium, Firefox, and WebKit:

```sh
nix develop .#nixos -c scripts/browser-test all -- -p wasm-junction-example-streams --lib
```

The native entry point and browser test call the same `run` function and compare identical output.
Everything used by the example lives here: `wit/` defines the interfaces, `guest/` is the isolated
component workspace, `build.rs` builds it without WASI, `src/` contains the host and middleware,
and `tests/` checks the executable's complete output.
