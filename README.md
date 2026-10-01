# wasm-junction

`wasm-junction` is an ergonomic Rust framework for building applications from WebAssembly
components. It is early in development and its API is not yet stable.

## Examples

- [`examples/greeter`](examples/greeter) calls a component that reads users from a host provider.
- [`examples/clock`](examples/clock) intercepts WASI clock calls with middleware.
- [`examples/translate`](examples/translate) routes calls from one component to either of two
  component providers.

Run all checks with:

```sh
nix develop -c pre-commit run --all-files --hook-stage pre-push
```
