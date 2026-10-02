# wasm-junction-jco

This crate runs WebAssembly components in browsers. It transpiles component bytes with jco at
load time, connects every import to wasm-junction's dispatcher, and creates a fresh instance for
each exported call.

The engine requires JavaScript Promise Integration (JSPI), available in current Chromium and
Firefox and in Safari 27 or newer. Generated modules are imported through Blob URLs, so a strict
Content Security Policy must allow `script-src blob:`. Transpiling happens on the application's
thread and can pause it for a few milliseconds during a load or reload.

Browser tests run in dedicated Workers on Chromium, Firefox, and WebKit. From the repository root:

```sh
nix develop -c scripts/browser-test all -- --workspace-tests
```
