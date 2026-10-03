# HTTP

This example sends a WASI Preview 3 HTTP request through middleware before network I/O starts.
The host starts a one-request server on `127.0.0.1` with an ephemeral port, enables networking for
the component, and registers both `wasi::provider()` and `wasi::http::provider()`.

`OriginPolicy` reads the method, authority, path, and headers supplied on the
`wasi:http/client@0.3.0.send` call. It allows only the loopback server, inserts an `x-secret`
header, and refuses a second origin. Refusal becomes the guest-visible `HTTP-request-denied`
error rather than a trap. The server proves that the rewritten header and streamed request body
reached the wire; the guest also reads the streamed response body and awaits its futures.

The guest uses `wasip3::http::client::send`, the same API shape used by ordinary Preview 3 HTTP
plugins, while its small exported interface comes from the local `wit/` directory. It is built for
`wasm32-unknown-unknown` and componentized without a WASI adapter.

Run it from the repository root:

```sh
nix develop -c cargo run -p wasm-junction-example-http
```

The output is deterministic even though the server chooses an ephemeral port:

```text
call host → http example:http/client@0.1.0.fetch()
call http → host wasi:http/client@0.3.0.send()
policy allow POST http://local/message
allowed: 200:hello from server
server received rewritten header and request body
call host → http example:http/client@0.1.0.fetch()
call http → host wasi:http/client@0.3.0.send()
policy deny POST http://blocked.invalid/message
blocked: denied
```

Everything needed to follow the example is in this directory: the WIT contract, isolated guest
workspace, build script, loopback server, trace and policy middleware, host, and output test.
