# Files

This example runs an ordinary Rust `wasm32-wasip2` guest that reads files with
`std::fs::read_to_string`. The host creates two directories under a fresh temporary root, writes a
`note.txt` into each, and exposes them to the guest as `/allowed` and `/blocked`. It never depends
on a path or file already present on the machine.

The application registers WASI explicitly and configures both directories per component:

```rust
app.configure(
    "files",
    WasiSettings::new()
        .preopen(allowed, "/allowed", Access::ReadOnly)
        .preopen(blocked, "/blocked", Access::ReadOnly),
)?;
```

Every filesystem call still passes through middleware. `open-at` carries the guest path of the
preopen that produced its descriptor after the declared WIT arguments. The policy allows
`/allowed` and refuses `/blocked`; it does not make its decision from the relative `note.txt` path.
The refusal becomes WASI `error-code.access`, which Rust reports to the guest as
`ErrorKind::PermissionDenied`.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-files
```

The output is deterministic:

```text
call host → files example:files/files@0.1.0.read()
call files → host wasi:filesystem/preopens@0.2.12.get-directories()
call files → host wasi:filesystem/types@0.2.12.[method]descriptor.open-at()
policy allow preopen /allowed
allowed: a readable note
call host → files example:files/files@0.1.0.read()
call files → host wasi:filesystem/preopens@0.2.12.get-directories()
call files → host wasi:filesystem/types@0.2.12.[method]descriptor.open-at()
policy deny preopen /blocked
blocked: PermissionDenied
```

The directory is self-contained: `wit/` is the component contract, `guest/` is its isolated Rust
workspace, `build.rs` builds the guest for `wasm32-wasip2`, `src/middleware.rs` contains the tracer
and policy, and the output test runs the complete example.
