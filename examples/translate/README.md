# Translate

This example routes a writer component's `translator` import to other WebAssembly components.
The translators are deterministic fakes: each has a tiny built-in dictionary and marks its output,
so no network service or credentials are needed.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-translate
```

## In the browser

Run the dedicated-Worker browser test in Chromium, Firefox, and WebKit:

```sh
nix develop -c scripts/browser-test all -- -p wasm-junction-example-translate --lib
```

The native entry point and the browser test call the same `run` function. In a browser, loading
each embedded component also transpiles it to JavaScript on the Worker's thread.

The executable demonstrates resolution at call time:

1. It loads `deepl`, then a `writer` that imports the translator interface. The first draft is
   served by `deepl`.
2. Loading `google` is refused because the writer would have two possible providers.
3. It links the writer to `deepl`, loads `google`, and writes another draft through `deepl`.
4. It changes the link to `google`, and the next draft uses `google` without reloading the writer.
5. It runs `App::check` to confirm that every import resolves cleanly.

Tracing middleware prints each caller and callee. A single draft visibly crosses both component
boundaries: `host → writer`, followed by `writer → deepl` or `writer → google`.

The files are intentionally self-contained:

- `wit/translate.wit` defines the imported translator interface, the writer interface, and both
  component worlds.
- `guest/` is an isolated Rust workspace containing the writer, fake DeepL, and fake Google
  components.
- `build.rs` builds all three guests for `wasm32-unknown-unknown` and componentizes them without
  WASI.
- `src/bindings.rs` generates typed host bindings from the WIT package.
- `src/trace.rs` contains the example's tracing middleware.
- `src/lib.rs` loads, links, calls, and checks the components.
- `src/main.rs` uses Tokio to run that target-neutral workflow and print its output.
- `tests/` runs the executable and checks its complete output.

The fake dictionary recognizes `hello` translated to Portuguese. Other inputs return an ordinary
WIT `result` error from the selected translator.
