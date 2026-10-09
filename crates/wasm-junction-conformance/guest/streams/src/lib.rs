//! Component used to exercise byte streams in both directions.

mod bindings {
    wit_bindgen::generate!({
        path: "../../stream-wit",
        world: "example:streams/streams@0.1.0",
        generate_all,
    });
}

use bindings::example::streams::host;
use wit_bindgen::StreamReader;

struct Component;

impl bindings::exports::example::streams::probe::Guest for Component {
    fn echo_bytes(bytes: Vec<u8>) -> Vec<u8> {
        bytes
    }

    async fn motd() -> String {
        text(host::motd().collect().await)
    }

    async fn audit() {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer.write_all(b"opened note\n".to_vec()).await;
        });
        host::audit(reader).await;
    }

    async fn optional() -> String {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer.write_all(b"nested import".to_vec()).await;
        });
        let stream = host::optional(Some(reader)).await.unwrap();
        text(stream.collect().await)
    }

    async fn echo_optional(bytes: Option<StreamReader<u8>>) -> Option<StreamReader<u8>> {
        bytes
    }

    async fn leave_open() {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer.write_all(b"written".to_vec()).await;
            host::checkpoint().await;
            let _ = writer.write_all(b"in flight".to_vec()).await;
            host::checkpoint().await;
            core::mem::forget(writer);
        });
        host::audit(reader).await;
    }

    async fn incremental() -> String {
        let mut stream = host::chunks();
        let (_, mut bytes) = stream.read(Vec::with_capacity(64)).await;
        host::advance();
        bytes.extend(stream.collect().await);
        text(bytes)
    }

    async fn drop_early() {
        drop(host::chunks());
        host::advance();
    }

    async fn accept(bytes: StreamReader<u8>) -> String {
        text(bytes.collect().await)
    }

    fn return_host() -> StreamReader<u8> {
        host::motd()
    }

    fn return_guest() -> StreamReader<u8> {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer.write_all(b"guest".to_vec()).await;
        });
        reader
    }

    async fn poison_streams() {
        let host_stream = host::chunks();
        let (mut writer, reader) = bindings::wit_stream::new();
        host::poison(reader).await;
        let _ = writer.write_all(b"after refusal".to_vec()).await;
        let _ = host_stream.collect().await;
        host::advance();
    }
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

bindings::export!(Component with_types_in bindings);
