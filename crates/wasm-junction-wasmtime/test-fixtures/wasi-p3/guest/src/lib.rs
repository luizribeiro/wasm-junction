wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
    generate_all,
});

use std::future::{Future, IntoFuture, poll_fn};
use std::task::Poll;

struct Component;

async fn join<A: Future, B: Future>(left: A, right: B) -> (A::Output, B::Output) {
    let mut left = std::pin::pin!(left);
    let mut right = std::pin::pin!(right);
    let mut left_result = None;
    let mut right_result = None;
    poll_fn(move |context| {
        if left_result.is_none()
            && let Poll::Ready(result) = left.as_mut().poll(context)
        {
            left_result = Some(result);
        }
        if right_result.is_none()
            && let Poll::Ready(result) = right.as_mut().poll(context)
        {
            right_result = Some(result);
        }
        match (left_result.take(), right_result.take()) {
            (Some(left), Some(right)) => Poll::Ready((left, right)),
            (left, right) => {
                left_result = left;
                right_result = right;
                Poll::Pending
            }
        }
    })
    .await
}

async fn write_stdout(bytes: Vec<u8>) -> bool {
    let (mut writer, reader) = wit_stream::new();
    let write = async move {
        assert!(writer.write_all(bytes).await.is_empty());
    };
    let completion = wasi::cli::stdout::write_via_stream(reader).into_future();
    let (_, result) = join(write, completion).await;
    result.is_ok()
}

async fn write_stderr(bytes: Vec<u8>) -> bool {
    let (mut writer, reader) = wit_stream::new();
    let write = async move {
        assert!(writer.write_all(bytes).await.is_empty());
    };
    let completion = wasi::cli::stderr::write_via_stream(reader).into_future();
    let (_, result) = join(write, completion).await;
    result.is_ok()
}

async fn cli_probe() -> String {
    let environment = wasi::cli::environment::get_environment();
    let arguments = wasi::cli::environment::get_arguments();
    let cwd = wasi::cli::environment::get_initial_cwd();
    let (input, completion) = wasi::cli::stdin::read_via_stream();
    let input = input.collect().await;
    let input_result = match completion.await {
        Ok(()) => "ok",
        Err(wasi::cli::types::ErrorCode::Io) => "io",
        Err(wasi::cli::types::ErrorCode::IllegalByteSequence) => "illegal-byte-sequence",
        Err(wasi::cli::types::ErrorCode::Pipe) => "pipe",
    };
    let stdout_ok = write_stdout(b"stdout".to_vec()).await;
    let stderr_ok = write_stderr(b"stderr".to_vec()).await;
    let terminals = (
        wasi::cli::terminal_stdin::get_terminal_stdin().is_some(),
        wasi::cli::terminal_stdout::get_terminal_stdout().is_some(),
        wasi::cli::terminal_stderr::get_terminal_stderr().is_some(),
    );
    format!(
        "{environment:?}|{arguments:?}|{cwd:?}|{}|{input_result}|{stdout_ok}|{stderr_ok}|{terminals:?}",
        input.len()
    )
}

fn filesystem_root() -> wasi::filesystem::types::Descriptor {
    wasi::filesystem::preopens::get_directories()
        .into_iter()
        .find(|(_, path)| path == "/data")
        .unwrap()
        .0
}

async fn directory_names() -> Result<Vec<String>, String> {
    let (entries, completion) = filesystem_root().read_directory();
    let entries = entries.collect().await;
    completion.await.map_err(|error| format!("{error:?}"))?;
    Ok(entries.into_iter().map(|entry| entry.name).collect())
}

async fn send_bytes(
    descriptor: &wasi::filesystem::types::Descriptor,
    bytes: &[u8],
    append: bool,
) {
    let (mut writer, reader) = wit_stream::new();
    let write = async move {
        assert!(writer.write_all(bytes.to_vec()).await.is_empty());
    };
    let completion = if append {
        descriptor.append_via_stream(reader)
    } else {
        descriptor.write_via_stream(reader, 0)
    };
    let (_, result) = join(write, completion.into_future()).await;
    result.unwrap();
}

async fn filesystem_probe() -> String {
    use wasi::filesystem::types::{DescriptorFlags, OpenFlags, PathFlags};

    let root = filesystem_root();
    let file = root
        .open_at(
            PathFlags::empty(),
            "note.txt".to_owned(),
            OpenFlags::CREATE | OpenFlags::TRUNCATE,
            DescriptorFlags::READ | DescriptorFlags::WRITE,
        )
        .await
        .unwrap();
    send_bytes(&file, b"hello", false).await;
    send_bytes(&file, b" world", true).await;
    let (bytes, completion) = file.read_via_stream(0);
    let bytes = bytes.collect().await;
    completion.await.unwrap();
    let names = directory_names().await.unwrap();
    format!("{}|{}", String::from_utf8(bytes).unwrap(), names.join(","))
}

async fn cover_filesystem() {
    use wasi::filesystem::types::{Advice, DescriptorFlags, NewTimestamp, OpenFlags, PathFlags};

    let _ = filesystem_probe().await;
    let root = filesystem_root();
    let _ = root.advise(0, 0, Advice::Normal).await;
    let _ = root.sync_data().await;
    let _ = root.get_flags().await;
    let _ = root.get_type().await;
    let _ = root.set_size(0).await;
    let _ = root
        .set_times(NewTimestamp::NoChange, NewTimestamp::NoChange)
        .await;
    let _ = root.sync().await;
    let _ = root.create_directory_at("remove-me".to_owned()).await;
    let _ = root.stat().await;
    let _ = root.stat_at(PathFlags::empty(), "note.txt".to_owned()).await;
    let _ = root
        .set_times_at(
            PathFlags::empty(),
            "note.txt".to_owned(),
            NewTimestamp::NoChange,
            NewTimestamp::NoChange,
        )
        .await;
    let _ = root
        .link_at(PathFlags::empty(), "missing".to_owned(), &root, "link".to_owned())
        .await;
    let _ = root
        .open_at(
            PathFlags::empty(),
            "note.txt".to_owned(),
            OpenFlags::empty(),
            DescriptorFlags::READ,
        )
        .await;
    let _ = root.readlink_at("missing".to_owned()).await;
    let _ = root.remove_directory_at("remove-me".to_owned()).await;
    let _ = root
        .rename_at("missing".to_owned(), &root, "renamed".to_owned())
        .await;
    let _ = root.symlink_at("missing".to_owned(), "symlink".to_owned()).await;
    let _ = root.unlink_file_at("symlink".to_owned()).await;
    let _ = root.is_same_object(&root).await;
    let _ = root.metadata_hash().await;
    let _ = root
        .metadata_hash_at(PathFlags::empty(), "note.txt".to_owned())
        .await;
}

fn loopback(port: u16) -> wasi::sockets::types::IpSocketAddress {
    wasi::sockets::types::IpSocketAddress::Ipv4(
        wasi::sockets::types::Ipv4SocketAddress {
            port,
            address: (127, 0, 0, 1),
        },
    )
}

async fn echo_socket(socket: wasi::sockets::types::TcpSocket) -> Result<String, String> {
    let (bytes, received) = socket.receive();
    let bytes = bytes.collect().await;
    received.await.map_err(|error| format!("{error:?}"))?;
    let text = String::from_utf8(bytes.clone()).map_err(|error| error.to_string())?;
    let (mut writer, reader) = wit_stream::new();
    let write = async move {
        assert!(writer.write_all(bytes).await.is_empty());
    };
    let sent = socket.send(reader).into_future();
    let (_, result) = join(write, sent).await;
    result.map_err(|error| format!("{error:?}"))?;
    Ok(text)
}

async fn tcp_echo(count: u8) -> Result<Vec<String>, String> {
    use wasi::sockets::types::{IpAddressFamily, TcpSocket};

    let listener = TcpSocket::create(IpAddressFamily::Ipv4)
        .map_err(|error| format!("{error:?}"))?;
    listener
        .bind(loopback(0))
        .map_err(|error| format!("{error:?}"))?;
    let mut accepted = listener
        .listen()
        .map_err(|error| format!("{error:?}"))?;
    let mut messages = Vec::new();
    for _ in 0..count {
        let socket = accepted
            .next()
            .await
            .ok_or_else(|| "accept stream closed".to_owned())?;
        messages.push(echo_socket(socket).await?);
    }
    Ok(messages)
}

async fn tcp_accept_batch(count: u8) -> u8 {
    use wasi::sockets::types::{IpAddressFamily, TcpSocket};

    let listener = TcpSocket::create(IpAddressFamily::Ipv4).unwrap();
    listener.bind(loopback(0)).unwrap();
    let mut accepted = listener.listen().unwrap();
    let (_, sockets) = accepted
        .read(Vec::with_capacity(usize::from(count)))
        .await;
    u8::try_from(sockets.len()).unwrap()
}

async fn drop_accept_stream() -> Result<(), String> {
    use wasi::sockets::types::{IpAddressFamily, TcpSocket};

    let listener = TcpSocket::create(IpAddressFamily::Ipv4)
        .map_err(|error| format!("{error:?}"))?;
    listener
        .bind(loopback(0))
        .map_err(|error| format!("{error:?}"))?;
    drop(
        listener
            .listen()
            .map_err(|error| format!("{error:?}"))?,
    );
    Ok(())
}

async fn tcp_stream_failures() -> Vec<String> {
    use wasi::sockets::types::{IpAddressFamily, TcpSocket};

    let socket = TcpSocket::create(IpAddressFamily::Ipv4).unwrap();
    let (writer, reader) = wit_stream::new();
    drop(writer);
    let send = socket.send(reader).into_future().await.unwrap_err();
    let (stream, completion) = socket.receive();
    drop(stream);
    let receive = completion.await.unwrap_err();
    vec![format!("{send:?}"), format!("{receive:?}")]
}

async fn cover_sockets() {
    use wasi::sockets::types::{IpAddressFamily, TcpSocket, UdpSocket};

    let tcp = TcpSocket::create(IpAddressFamily::Ipv4).unwrap();
    tcp.bind(loopback(0)).unwrap();
    let _ = tcp.get_local_address();
    let _ = tcp.get_remote_address();
    let _ = tcp.get_is_listening();
    let _ = tcp.get_address_family();
    let _ = tcp.set_listen_backlog_size(1);
    let _ = tcp.get_keep_alive_enabled();
    let _ = tcp.set_keep_alive_enabled(true);
    let _ = tcp.get_keep_alive_idle_time();
    let _ = tcp.set_keep_alive_idle_time(1);
    let _ = tcp.get_keep_alive_interval();
    let _ = tcp.set_keep_alive_interval(1);
    let _ = tcp.get_keep_alive_count();
    let _ = tcp.set_keep_alive_count(1);
    let _ = tcp.get_hop_limit();
    let _ = tcp.set_hop_limit(1);
    let _ = tcp.get_receive_buffer_size();
    let _ = tcp.set_receive_buffer_size(4096);
    let _ = tcp.get_send_buffer_size();
    let _ = tcp.set_send_buffer_size(4096);
    drop(tcp.listen());

    let streams = TcpSocket::create(IpAddressFamily::Ipv4).unwrap();
    let (writer, reader) = wit_stream::new();
    drop(writer);
    let _ = streams.send(reader).into_future().await;
    let (bytes, received) = streams.receive();
    drop(bytes);
    let _ = received.await;
    let connector = TcpSocket::create(IpAddressFamily::Ipv4).unwrap();
    let _ = connector.connect(loopback(0)).await;

    let udp = UdpSocket::create(IpAddressFamily::Ipv4).unwrap();
    udp.bind(loopback(0)).unwrap();
    let local = udp.get_local_address().unwrap();
    let _ = udp.get_remote_address();
    let _ = udp.get_address_family();
    let _ = udp.get_unicast_hop_limit();
    let _ = udp.set_unicast_hop_limit(1);
    let _ = udp.get_receive_buffer_size();
    let _ = udp.set_receive_buffer_size(4096);
    let _ = udp.get_send_buffer_size();
    let _ = udp.set_send_buffer_size(4096);
    udp.send(b"coverage".to_vec(), Some(local)).await.unwrap();
    let _ = udp.receive().await;
    let _ = udp.connect(loopback(9));
    let _ = udp.disconnect();
    let _ = wasi::sockets::ip_name_lookup::resolve_addresses("localhost".to_owned()).await;
}

async fn udp_receive() -> Result<String, String> {
    use wasi::sockets::types::{IpAddressFamily, UdpSocket};

    let socket = UdpSocket::create(IpAddressFamily::Ipv4)
        .map_err(|error| format!("{error:?}"))?;
    socket
        .bind(loopback(0))
        .map_err(|error| format!("{error:?}"))?;
    let (bytes, remote) = socket
        .receive()
        .await
        .map_err(|error| format!("{error:?}"))?;
    socket
        .send(bytes.clone(), Some(remote))
        .await
        .map_err(|error| format!("{error:?}"))?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

impl exports::test::wasi_p3::probe::Guest for Component {
    async fn coverage() -> String {
        let _ = cli_probe().await;
        let resolution = wasi::clocks::monotonic_clock::get_resolution();
        let first = wasi::clocks::monotonic_clock::now();
        wasi::clocks::monotonic_clock::wait_for(1_000_000).await;
        let second = wasi::clocks::monotonic_clock::now();
        wasi::clocks::monotonic_clock::wait_until(second).await;
        let system = wasi::clocks::system_clock::now();
        let system_resolution = wasi::clocks::system_clock::get_resolution();
        let random = wasi::random::random::get_random_bytes(4);
        let insecure = wasi::random::insecure::get_insecure_random_bytes(5);
        let _ = wasi::random::random::get_random_u64();
        let _ = wasi::random::insecure::get_insecure_random_u64();
        let _ = wasi::random::insecure_seed::get_insecure_seed();
        format!(
            "{}|{}|{}|{}|{}|{}",
            second >= first,
            system.seconds > 0,
            random.len(),
            insecure.len(),
            resolution > 0,
            system_resolution > 0,
        )
    }

    async fn cli() -> String {
        cli_probe().await
    }

    async fn drop_output_completion() -> Vec<String> {
        let (writer, reader) = wit_stream::new();
        drop(writer);
        drop(wasi::cli::stdout::write_via_stream(reader));
        let arguments = wasi::cli::environment::get_arguments();
        wasi::clocks::monotonic_clock::wait_for(1).await;
        arguments
    }

    async fn filesystem() -> String {
        filesystem_probe().await
    }

    async fn list_directory() -> Result<Vec<String>, String> {
        directory_names().await
    }

    async fn drop_directory_listing() -> Vec<String> {
        let (entries, completion) = filesystem_root().read_directory();
        drop(entries);
        drop(completion);
        directory_names().await.unwrap()
    }

    async fn filesystem_coverage() {
        cover_filesystem().await;
    }

    async fn tcp_echo(count: u8) -> Result<Vec<String>, String> {
        tcp_echo(count).await
    }

    async fn tcp_accept_batch(count: u8) -> u8 {
        tcp_accept_batch(count).await
    }

    async fn tcp_stream_failures() -> Vec<String> {
        tcp_stream_failures().await
    }

    async fn drop_accept_stream() {
        drop_accept_stream().await.unwrap();
    }

    async fn udp_receive() -> Result<String, String> {
        udp_receive().await
    }

    async fn lookup_localhost() -> Result<u32, String> {
        wasi::sockets::ip_name_lookup::resolve_addresses("localhost".to_owned())
            .await
            .map(|addresses| u32::try_from(addresses.len()).unwrap())
            .map_err(|error| format!("{error:?}"))
    }

    async fn sockets_coverage() {
        cover_sockets().await;
    }

    async fn exit_success() {
        wasi::cli::exit::exit(Ok(()));
    }

    async fn exit_code() {
        wasi::cli::exit::exit_with_code(7);
    }
}

export!(Component);
