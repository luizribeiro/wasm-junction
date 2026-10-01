//! End-to-end byte-stream checks for the native engine.

use std::sync::{Arc, Mutex};

use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, Component, InputStream, Provided, Provider, Val,
    Vals,
};
use wasm_junction_conformance::{STREAM_HOST, STREAM_PROBE, StreamHost, stream_component};
use wasm_junction_wasmtime::WasmtimeEngine;

#[derive(Default)]
struct RetainedStream {
    first: Vec<u8>,
    input: Option<InputStream>,
}

#[derive(Clone, Default)]
struct RetainHost(Arc<Mutex<RetainedStream>>);

impl Provider for RetainHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            let [value] = <[_; 1]>::try_from(call.args)
                .map_err(|_| CallError::trap("audit expects one stream"))?;
            let mut input = InputStream::try_from(value)?;
            let first = input
                .read()
                .await
                .map_err(|error| CallError::trap(error.to_string()))?
                .ok_or_else(|| CallError::trap("guest stream closed before its first chunk"))?;
            *self.0.lock().unwrap() = RetainedStream {
                first,
                input: Some(input),
            };
            Ok(Vec::new())
        })
    }
}

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

#[test]
fn open_guest_stream_is_aborted_when_its_store_ends() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = RetainHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(Provided::new(STREAM_HOST, host.clone()))
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(stream_component())
                    .unwrap()
                    .named("streams"),
            )
            .await
            .unwrap();
            app.call("streams", STREAM_PROBE, "leave-open", Vec::new())
                .await
                .unwrap();

            let mut input = {
                let mut retained = host.0.lock().unwrap();
                assert_eq!(retained.first, b"written");
                retained.input.take().unwrap()
            };
            assert_eq!(input.read().await.unwrap(), Some(b"in flight".to_vec()));
            let error = input.read().await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "stream was aborted when its invocation ended"
            );
        });
}
