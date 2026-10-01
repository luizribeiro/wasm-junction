//! End-to-end byte-stream checks for the native engine.

use wasm_junction::{App, Component, Val};
use wasm_junction_conformance::{STREAM_PROBE, StreamHost, stream_component};
use wasm_junction_wasmtime::WasmtimeEngine;

#[test]
fn host_streams_reach_the_guest_and_close_on_early_drop() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = StreamHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(host.clone().provided())
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(stream_component())
                    .unwrap()
                    .named("streams"),
            )
            .await
            .unwrap();

            let output = app
                .call("streams", STREAM_PROBE, "motd", Vec::new())
                .await
                .unwrap();
            assert_eq!(output, [Val::from("Have a good day.")]);
            app.call("streams", STREAM_PROBE, "audit", Vec::new())
                .await
                .unwrap();
            assert_eq!(host.audit(), b"opened note\n");
            app.call("streams", STREAM_PROBE, "drop-early", Vec::new())
                .await
                .unwrap();
            assert!(host.reader_closed());
            assert_eq!(
                host.write_error().as_deref(),
                Some("stream reader is closed")
            );
        });
}
