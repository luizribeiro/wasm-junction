//! End-to-end byte-stream checks for the native engine.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Component, InputStream,
    OutputStream, Provided, Provider, Resource, StreamHandle, Val, Vals,
};
use wasm_junction_conformance::{
    PoisonHost, RESOURCE_CLIENT, RESOURCE_HOST, ResourceHost, RetainHost, STREAM_PROBE, StreamHost,
    VALUE_STREAM_PROBE, ValueStreamHost, named_value_stream_component, resource_stream_component,
    run_streams, stream_component, value_stream_component,
};
use wasm_junction_wasmtime::WasmtimeEngine;
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

#[derive(Default)]
struct InvalidResourceStreamState {
    active: AtomicUsize,
    constructor_calls: AtomicUsize,
    read: Mutex<Option<tokio::task::JoinHandle<String>>>,
}

#[derive(Clone, Default)]
struct InvalidResourceStreamHost(Arc<InvalidResourceStreamState>);

impl Provider for InvalidResourceStreamHost {
    fn call<'a>(
        &'a self,
        _cx: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            match call.function.as_ref() {
                "[constructor]session" => {
                    let call = self.0.constructor_calls.fetch_add(1, Ordering::Relaxed);
                    let id = match call {
                        0 | 1 => 0,
                        _ => 1,
                    };
                    if call != 1 {
                        self.0.active.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(vec![Val::Resource(Resource::owned(
                        RESOURCE_HOST,
                        "session",
                        id,
                    ))])
                }
                "sessions" => {
                    self.0.active.fetch_add(1, Ordering::Relaxed);
                    let valid = Val::Resource(Resource::owned(RESOURCE_HOST, "session", 0));
                    let items = vec![
                        valid,
                        Val::Resource(Resource::owned("example:foreign/host@1.0.0", "session", 7)),
                    ];
                    Ok(vec![OutputStream::from_items(items).into()])
                }
                "[method]session.profile" => Ok(vec![Val::from("profile:recovered")]),
                "accept-sessions" => {
                    let [Val::Stream(stream)] = <[_; 1]>::try_from(call.args)
                        .map_err(|_| CallError::trap("accept-sessions expects one stream"))?
                    else {
                        return Err(CallError::trap("accept-sessions expects a stream"));
                    };
                    let input = InputStream::<Resource>::from_handle(stream)?;
                    let state = self.0.clone();
                    *self.0.read.lock().unwrap() = Some(tokio::spawn(async move {
                        match input.read_all().await {
                            Ok(items) => {
                                state.active.fetch_sub(items.len(), Ordering::Relaxed);
                                "stream completed".to_owned()
                            }
                            Err(error) => error.to_string(),
                        }
                    }));
                    Ok(vec![Val::from("reading")])
                }
                _ => Err(CallError::trap("unexpected resource call")),
            }
        })
    }

    fn drop_resource(&self, _cx: &CallContext, resource: Resource) -> Result<(), CallError> {
        assert_eq!(resource.interface(), RESOURCE_HOST);
        assert_eq!(resource.name(), "session");
        self.0.active.fetch_sub(1, Ordering::Relaxed);
        Ok(())
    }
}

#[test]
fn bidirectional_streams_match_the_engine_neutral_trace() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(run_streams(WasmtimeEngine::new().unwrap()))
        .unwrap();
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

            let nested_import = app
                .call("streams", STREAM_PROBE, "optional", Vec::new())
                .await
                .unwrap();
            assert_eq!(nested_import, [Val::from("nested import")]);

            let mut nested_export = app
                .call(
                    "streams",
                    STREAM_PROBE,
                    "echo-optional",
                    vec![Val::Option(Some(Box::new(
                        OutputStream::from_bytes(b"nested export").into(),
                    )))],
                )
                .await
                .unwrap();
            let Val::Option(Some(stream)) = nested_export.remove(0) else {
                panic!("echo-optional did not return a stream option");
            };
            let input = InputStream::try_from(*stream).unwrap();
            assert_eq!(input.read_all().await.unwrap(), b"nested export");

            let accepted = app
                .call(
                    "streams",
                    STREAM_PROBE,
                    "accept",
                    vec![OutputStream::from_bytes(b"export argument").into()],
                )
                .await
                .unwrap();
            assert_eq!(accepted, [Val::from("export argument")]);

            let mut returned = app
                .call("streams", STREAM_PROBE, "return-host", Vec::new())
                .await
                .unwrap();
            let input = InputStream::try_from(returned.remove(0)).unwrap();
            assert_eq!(input.read_all().await.unwrap(), b"Have a good day.");

            let error = app
                .call("streams", STREAM_PROBE, "return-guest", Vec::new())
                .await
                .unwrap_err();
            assert_eq!(error.kind(), CallErrorKind::Refused);
            assert!(error.to_string().contains("store ends with each call"));

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
            app.call("streams", STREAM_PROBE, "leave-open", Vec::new())
                .await
                .unwrap();

            assert_eq!(host.first(), b"written");
            let mut input = host.take_input().unwrap();
            assert_eq!(input.read().await.unwrap(), Some(b"in flight".to_vec()));
            let error = input.read().await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "stream was aborted when its invocation ended"
            );
        });
}

#[test]
fn string_streams_cross_imports_and_exports_in_order() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = ValueStreamHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(host.clone().provided())
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(value_stream_component())
                    .unwrap()
                    .named("value-streams"),
            )
            .await
            .unwrap();

            let output = app
                .call(
                    "value-streams",
                    VALUE_STREAM_PROBE,
                    "exchange-strings",
                    Vec::new(),
                )
                .await
                .unwrap();
            assert_eq!(
                output,
                [Val::List(vec![
                    Val::from("host one"),
                    Val::from("host two")
                ])]
            );
            assert_eq!(host.strings(), ["guest one", "guest two"]);

            app.call(
                "value-streams",
                VALUE_STREAM_PROBE,
                "send-lists",
                Vec::new(),
            )
            .await
            .unwrap();
            assert_eq!(host.lists(), [vec![1, 2], vec![3]]);

            let input = OutputStream::from_items(["first".to_owned(), "second".to_owned()]);
            assert_eq!(
                app.call(
                    "value-streams",
                    VALUE_STREAM_PROBE,
                    "echo-strings",
                    vec![input.into()],
                )
                .await
                .unwrap(),
                [Val::List(vec![Val::from("first"), Val::from("second")])]
            );

            let mut returned = app
                .call(
                    "value-streams",
                    VALUE_STREAM_PROBE,
                    "return-host-strings",
                    Vec::new(),
                )
                .await
                .unwrap();
            let Val::Stream(stream) = returned.remove(0) else {
                panic!("expected string stream")
            };
            assert_eq!(
                InputStream::<String>::from_handle(stream)
                    .unwrap()
                    .read_all()
                    .await
                    .unwrap(),
                ["host one", "host two"]
            );
        });
}

#[test]
fn nested_value_streams_round_trip_and_middleware_filters_items() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = ValueStreamHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(host.provided())
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(value_stream_component())
                    .unwrap()
                    .named("value-streams"),
            )
            .await
            .unwrap();

            let lists = OutputStream::from_items([vec![1_u32, 2], vec![3]]);
            let lists = lists.__into_handle_with(|items| {
                Ok(Val::List(items.into_iter().map(Val::U32).collect()))
            });
            assert_eq!(
                app.call(
                    "value-streams",
                    VALUE_STREAM_PROBE,
                    "echo-lists",
                    vec![Val::Stream(lists)],
                )
                .await
                .unwrap(),
                [Val::List(vec![
                    Val::List(vec![Val::U32(1), Val::U32(2)]),
                    Val::List(vec![Val::U32(3)]),
                ])]
            );

            let nested = vec![
                vec![Some("one".to_owned()), None],
                vec![Some("two".to_owned())],
            ];
            let stream = OutputStream::from_items(nested.clone());
            let stream = stream.__into_handle_with(|items| {
                Ok(Val::List(
                    items
                        .into_iter()
                        .map(|item| Val::Option(item.map(|value| Box::new(value.into()))))
                        .collect(),
                ))
            });
            let expected = Val::List(
                nested
                    .into_iter()
                    .map(|items| {
                        Val::List(
                            items
                                .into_iter()
                                .map(|item| Val::Option(item.map(|value| Box::new(value.into()))))
                                .collect(),
                        )
                    })
                    .collect(),
            );
            assert_eq!(
                app.call(
                    "value-streams",
                    VALUE_STREAM_PROBE,
                    "echo-nested",
                    vec![Val::Stream(stream)],
                )
                .await
                .unwrap(),
                [expected]
            );

            let stream = StreamHandle::from(OutputStream::from_items([
                "visible".to_owned(),
                "hidden".to_owned(),
                "last".to_owned(),
            ]))
            .filter_items(|item| item != &Val::from("hidden"));
            assert_eq!(
                app.call(
                    "value-streams",
                    VALUE_STREAM_PROBE,
                    "echo-strings",
                    vec![Val::Stream(stream)],
                )
                .await
                .unwrap(),
                [Val::List(vec![Val::from("visible"), Val::from("last")])]
            );
        });
}

#[test]
fn resource_stream_items_are_checked_as_invocation_handles() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = ResourceHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(host.clone().provided())
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(resource_stream_component())
                    .unwrap()
                    .named("resources"),
            )
            .await
            .unwrap();

            assert_eq!(
                app.call(
                    "resources",
                    RESOURCE_CLIENT,
                    "use-host-sessions",
                    Vec::new(),
                )
                .await
                .unwrap(),
                [Val::List(vec![
                    Val::from("profile:Ada"),
                    Val::from("profile:Grace")
                ])]
            );
            assert_eq!(host.active_resources(), 0);

            let foreign = OutputStream::from_items([Resource::owned(
                "example:foreign/host@1.0.0",
                "session",
                7,
            )]);
            let error = app
                .call(
                    "resources",
                    RESOURCE_CLIENT,
                    "use-sessions",
                    vec![foreign.into()],
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind(), CallErrorKind::Refused);
            assert!(error.to_string().contains("does not import resource"));
            assert!(
                app.call(
                    "resources",
                    RESOURCE_CLIENT,
                    "use-host-sessions",
                    Vec::new()
                )
                .await
                .is_ok()
            );
        });
}

#[test]
fn later_invalid_resource_stream_item_cleans_earlier_items() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = InvalidResourceStreamHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(Provided::new(RESOURCE_HOST, host.clone()))
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(resource_stream_component())
                    .unwrap()
                    .named("resources"),
            )
            .await
            .unwrap();

            let error = app
                .call(
                    "resources",
                    RESOURCE_CLIENT,
                    "use-host-sessions",
                    Vec::new(),
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind(), CallErrorKind::Refused);
            assert!(error.to_string().contains("does not import resource"));
            assert_eq!(host.0.active.load(Ordering::Relaxed), 0);
        });
}

#[test]
fn guest_resource_conversion_failure_aborts_the_host_stream() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = InvalidResourceStreamHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(Provided::new(RESOURCE_HOST, host.clone()))
                .build()
                .unwrap();
            app.load(
                Component::from_bytes(resource_stream_component())
                    .unwrap()
                    .named("resources"),
            )
            .await
            .unwrap();

            let error = app
                .call(
                    "resources",
                    RESOURCE_CLIENT,
                    "send-invalid-sessions",
                    Vec::new(),
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind(), CallErrorKind::Refused);
            assert!(error.to_string().contains("is no longer owned"));
            let read = host.0.read.lock().unwrap().take().unwrap();
            assert_eq!(
                read.await.unwrap(),
                "stream was aborted when its invocation ended"
            );
            assert_eq!(host.0.active.load(Ordering::Relaxed), 0);
            assert_eq!(
                app.call("resources", RESOURCE_CLIENT, "run", vec![Val::Bool(false)])
                    .await
                    .unwrap(),
                [Val::from("profile:recovered")]
            );
            assert_eq!(host.0.active.load(Ordering::Relaxed), 0);
        });
}

#[test]
fn unsupported_stream_items_are_refused_at_load_without_poisoning_the_app() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = StreamHost::default();
            let app = App::builder()
                .engine(WasmtimeEngine::new().unwrap())
                .provide(host.provided())
                .build()
                .unwrap();
            let cases = [
                (
                    "named-result",
                    "interface host { record note { text: string } values: func() -> stream<note>; } world test { import host; }",
                    "record",
                ),
                (
                    "named-export",
                    "interface probe { record note { text: string } accept: func(values: stream<note>); } world test { export probe; }",
                    "record",
                ),
                (
                    "named-nested",
                    "interface host { record note { text: string } accept: func(values: stream<list<note>>); } world test { import host; }",
                    "record",
                ),
                (
                    "tuple-result",
                    "interface host { values: func() -> stream<tuple<u32>>; } world test { import host; }",
                    "tuple",
                ),
                (
                    "tuple-export",
                    "interface probe { accept: func(values: stream<tuple<u32>>); } world test { export probe; }",
                    "tuple",
                ),
                (
                    "tuple-nested",
                    "interface host { accept: func(values: stream<option<tuple<u32>>>); } world test { import host; }",
                    "tuple",
                ),
                (
                    "result-item",
                    "interface host { accept: func(values: stream<result<string, string>>); } world test { import host; }",
                    "result",
                ),
            ];
            let error = app
                .load(
                    Component::from_bytes(named_value_stream_component())
                        .unwrap()
                        .named("named-record"),
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("stream item type `record`"));
            for (name, wit, shape) in cases {
                let error = app
                    .load(
                        Component::from_bytes(unsupported_stream_component(wit))
                            .unwrap()
                            .named(name),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(
                    error.to_string(),
                    format!(
                        "component compilation failed: stream item type `{shape}` is not supported on Wasmtime; supported item types are scalar values, strings, resources, and lists or options nested up to two layers"
                    )
                );
            }

            app.load(
                Component::from_bytes(stream_component())
                    .unwrap()
                    .named("streams"),
            )
            .await
            .unwrap();
            assert!(
                app.call("streams", STREAM_PROBE, "motd", Vec::new())
                    .await
                    .is_ok()
            );
        });
}

fn unsupported_stream_component(body: &str) -> Vec<u8> {
    let wit = format!("package example:streams@0.1.0; {body}");
    let mut resolve = Resolve::default();
    let package = resolve.push_str("unsupported.wit", &wit).unwrap();
    let world = resolve.select_world(&[package], Some("test")).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

#[test]
fn poisoned_invocation_blocks_further_stream_effects() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let host = PoisonHost::default();
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
            let error = app
                .call("streams", STREAM_PROBE, "poison-streams", Vec::new())
                .await
                .unwrap_err();
            assert_eq!(error.kind(), CallErrorKind::Refused);
            assert_eq!(error.to_string(), "stream refused");

            let mut guest = host.take_guest().unwrap();
            let source = host.take_source().unwrap();
            assert_eq!(
                guest.read().await.unwrap_err().to_string(),
                "stream was aborted when its invocation ended"
            );
            assert_eq!(
                source.write(b"late").await.unwrap_err().to_string(),
                "stream reader is closed"
            );
            assert_eq!(host.advances(), 0);
        });
}

#[test]
fn guest_reads_the_first_chunk_before_requesting_the_second() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let host = StreamHost::default();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    runtime
        .block_on(
            app.load(
                Component::from_bytes(stream_component())
                    .unwrap()
                    .named("streams"),
            ),
        )
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    let _worker = std::thread::spawn(move || {
        let result = runtime.block_on(app.call("streams", STREAM_PROBE, "incremental", Vec::new()));
        sender.send(result).unwrap();
    });

    let result = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("incremental stream deadlocked the current-thread Tokio runtime")
        .unwrap();
    assert_eq!(result, [Val::from("first second")]);
    assert!(host.advanced());
}
