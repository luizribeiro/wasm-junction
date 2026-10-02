# Clock

This example runs an ordinary Rust `wasm32-wasip2` guest with deterministic WASI settings. The
guest reads `GREETING` through `wasi:cli/environment` and the current time through
`wasi:clocks/wall-clock`. Both calls pass through the same middleware as the exported `run` call.
The application registers WASI explicitly with `.provide(wasi::provider())`.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-clock
```

The output is deterministic:

```text
call host → clock example:clock/clock@0.1.0.run()
call clock → host wasi:cli/environment@0.2.12.get-environment()
call clock → host wasi:clocks/wall-clock@0.2.12.now()
Hello from WASI at 1700000000.123456789
```

The directory is self-contained: `wit/` is the component contract, `guest/` is its isolated Rust
workspace, `build.rs` builds it normally for `wasm32-wasip2`, and `src/middleware.rs` contains the
tracer and fixed-clock middleware used by the host. The output test runs the complete example.
