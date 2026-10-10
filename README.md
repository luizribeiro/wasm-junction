# wasm-junction

`wasm-junction` is an ergonomic Rust framework for building applications from WebAssembly
components. It runs natively with Wasmtime and in Chromium, Firefox, and Safari 27 or newer with
JSPI. The default engine is selected automatically: jco on `wasm32-unknown-unknown`, and Wasmtime
on native targets. The project is early in development and its API is not yet stable.

## Examples

- [`examples/greeter`](examples/greeter) calls a component that reads users from a host provider.
- [`examples/clock`](examples/clock) intercepts WASI clock calls with middleware.
- [`examples/translate`](examples/translate) routes calls from one component to either of two
  component providers.
- [`examples/audit`](examples/audit) combines host resources, byte streams, and per-call data.
- [`examples/reload`](examples/reload) replaces a component while an earlier call is still running.
- [`examples/streams`](examples/streams) redacts and filters streams in middleware.

Run all checks with:

```sh
nix develop -c pre-commit run --all-files --hook-stage pre-push
```

On NixOS, use `nix develop .#nixos` instead, which supplies browsers that NixOS can run.
