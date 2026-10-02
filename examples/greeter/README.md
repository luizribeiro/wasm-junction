# Greeter

This example assembles a WebAssembly component from an ordinary Rust host. The host provides an
in-memory user directory, middleware traces calls in both directions, and the guest chooses a
greeting from each user's language.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-greeter
```

The files are intentionally self-contained:

- `wit/greeter.wit` defines the imported `users` interface, exported `greeter` interface, and
  component world.
- `guest/` is an isolated Rust workspace for the component implementation.
- `build.rs` builds that guest for `wasm32-unknown-unknown` and componentizes it without WASI.
- `src/bindings.rs` generates the host traits, providers, typed call views, and typed handles from
  the WIT package with `wasm_junction::bindgen!`.
- `src/trace.rs` is the example's tracing middleware.
- `src/lib.rs` provides users, builds the app, loads the component, and makes typed calls.
- `src/main.rs` uses Tokio to run that target-neutral workflow and print its output.
- `tests/` runs the executable and checks its complete output.

Call `.engine(…)` on the builder to override the target's default engine.
