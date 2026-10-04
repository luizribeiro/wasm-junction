wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
});

struct Component;

impl exports::test::wasi::environment::Guest for Component {
    fn read(name: String) -> Option<String> {
        std::env::var(name).ok()
    }

    fn arguments() -> Vec<String> {
        std::env::args().collect()
    }

    fn current_directory() -> Option<String> {
        wasi::cli::environment::initial_cwd()
    }

    fn wall_time() -> (u64, u32) {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        (elapsed.as_secs(), elapsed.subsec_nanos())
    }

    fn monotonic_time() -> u64 {
        wasi::clocks::monotonic_clock::now()
    }

    fn start_timer() {
        drop(wasi::clocks::monotonic_clock::subscribe_duration(0));
    }

    fn sleep() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    fn coverage() {
        let _ = wasi::cli::environment::get_environment();
        let _ = wasi::cli::environment::get_arguments();
        let _ = wasi::cli::environment::initial_cwd();
        let _ = wasi::clocks::wall_clock::now();
        let _ = wasi::clocks::wall_clock::resolution();
        let _ = wasi::clocks::monotonic_clock::now();
        let _ = wasi::clocks::monotonic_clock::resolution();
        drop(wasi::clocks::monotonic_clock::subscribe_instant(0));
        drop(wasi::clocks::monotonic_clock::subscribe_duration(0));
        let pollable = wasi::clocks::monotonic_clock::subscribe_duration(0);
        let _ = pollable.ready();
        pollable.block();
        let _ = wasi::io::poll::poll(&[&pollable]);
        let _ = wasi::random::random::get_random_bytes(4);
        let _ = wasi::random::random::get_random_u64();
        let _ = wasi::random::insecure::get_insecure_random_bytes(5);
        let _ = wasi::random::insecure::get_insecure_random_u64();
        let _ = wasi::random::insecure_seed::insecure_seed();
        drop(wasi::cli::terminal_stdin::get_terminal_stdin());
        drop(wasi::cli::terminal_stdout::get_terminal_stdout());
        drop(wasi::cli::terminal_stderr::get_terminal_stderr());

        let input = wasi::cli::stdin::get_stdin();
        let _ = input.read(0);
        let _ = input.blocking_read(0);
        let _ = input.skip(0);
        let _ = input.blocking_skip(0);
        drop(input.subscribe());

        let output = wasi::cli::stdout::get_stdout();
        drop(wasi::cli::stderr::get_stderr());
        let _ = output.check_write();
        let refused = output.write(&[0xfa]);
        if let Err(wasi::io::streams::StreamError::LastOperationFailed(error)) = refused {
            let _ = wasi::filesystem::types::filesystem_error_code(&error);
            let _ = error.to_debug_string();
            drop(error);
        }
        let _ = output.blocking_write_and_flush(&[]);
        let _ = output.flush();
        let _ = output.blocking_flush();
        drop(output.subscribe());
        let _ = output.check_write();
        let _ = output.write_zeroes(0);
        let _ = output.blocking_write_zeroes_and_flush(0);
        let _ = output.splice(&input, 0);
        let _ = output.blocking_splice(&input, 0);
    }

    fn differential() -> String {
        let environment = wasi::cli::environment::get_environment();
        let arguments = wasi::cli::environment::get_arguments();
        let cwd = wasi::cli::environment::initial_cwd();
        let wall = wasi::clocks::wall_clock::resolution();
        let monotonic = wasi::clocks::monotonic_clock::resolution();
        let wall_now = wasi::clocks::wall_clock::now();
        let first = wasi::clocks::monotonic_clock::now();
        let second = wasi::clocks::monotonic_clock::now();
        let warmup = wasi::clocks::monotonic_clock::subscribe_duration(50_000_000);
        drop(wasi::io::poll::poll(&[&warmup]));
        let timer_start = wasi::clocks::monotonic_clock::now();
        let short = wasi::clocks::monotonic_clock::subscribe_duration(20_000_000);
        let far = wasi::clocks::monotonic_clock::subscribe_instant(
            timer_start.saturating_add(5_000_000_000),
        );
        let ready = wasi::io::poll::poll(&[&short, &far]);
        let timer_elapsed =
            wasi::clocks::monotonic_clock::now().saturating_sub(timer_start);
        let timers_ordered = ready == [0]
            && timer_elapsed >= 10_000_000
            && timer_elapsed < 5_000_000_000;
        let input = wasi::cli::stdin::get_stdin();
        let output = wasi::cli::stdout::get_stdout();
        let stream = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            input.read(0),
            input.skip(0),
            input.subscribe().ready(),
            output.check_write(),
            output.write_zeroes(0),
            output.flush(),
            output.blocking_flush(),
            output.splice(&input, 0),
            output.blocking_write_and_flush(&[]),
            output.subscribe().ready(),
        );
        let random = wasi::random::random::get_random_bytes(4);
        let insecure = wasi::random::insecure::get_insecure_random_bytes(5);
        let _ = wasi::random::random::get_random_u64();
        let _ = wasi::random::insecure::get_insecure_random_u64();
        let _ = wasi::random::insecure_seed::insecure_seed();
        let terminals = (
            wasi::cli::terminal_stdin::get_terminal_stdin().is_some(),
            wasi::cli::terminal_stdout::get_terminal_stdout().is_some(),
            wasi::cli::terminal_stderr::get_terminal_stderr().is_some(),
        );
        format!(
            "{environment:?}|{arguments:?}|{cwd:?}|{}:{}|{monotonic}|{}|{}|{timers_ordered}|{stream}|{}:{}|{terminals:?}",
            wall.seconds,
            wall.nanoseconds,
            wall_now.seconds > 0,
            second >= first,
            random.len(),
            insecure.len(),
        )
    }

    fn refused_write() -> String {
        let output = wasi::cli::stdout::get_stdout();
        let _ = output.check_write();
        match output.write(b"refuse") {
            Err(wasi::io::streams::StreamError::LastOperationFailed(error)) => {
                error.to_debug_string()
            }
            other => format!("unexpected result: {other:?}"),
        }
    }

    fn input_read() -> String {
        let input = wasi::cli::stdin::get_stdin();
        match input.read(0) {
            Err(wasi::io::streams::StreamError::LastOperationFailed(error)) => {
                error.to_debug_string()
            }
            other => format!("unexpected result: {other:?}"),
        }
    }

    fn rewritten_write() -> bool {
        let output = wasi::cli::stdout::get_stdout();
        output
            .blocking_write_and_flush(&vec![0; 4097])
            .is_ok()
    }

    fn splice_first() {
        let input = wasi::cli::stdin::get_stdin();
        let output = wasi::cli::stdout::get_stdout();
        let _ = output.splice(&input, 0);
    }

    fn stdout_channel() {
        drop(wasi::cli::stdout::get_stdout());
    }

    fn random_bytes() -> Vec<u8> {
        wasi::random::random::get_random_bytes(4)
    }

    fn exit_success() {
        wasi::cli::exit::exit(Ok(()));
    }

    fn exit_code() {
        wasi::cli::exit::exit_with_code(7);
    }

    fn terminal_handles() {
        drop(wasi::cli::terminal_stdin::get_terminal_stdin());
        drop(wasi::cli::terminal_stdout::get_terminal_stdout());
        drop(wasi::cli::terminal_stderr::get_terminal_stderr());
    }

    fn benchmark_write(bytes: Vec<u8>) -> Vec<u8> {
        let output = wasi::cli::stdout::get_stdout();
        for chunk in bytes.chunks(4096) {
            let permit = output.check_write().unwrap();
            assert!(permit >= chunk.len() as u64);
            output.write(chunk).unwrap();
        }
        output.blocking_flush().unwrap();
        bytes
    }

    fn filesystem_coverage() {
        use wasi::filesystem::types::{
            Advice, DescriptorFlags, NewTimestamp, OpenFlags, PathFlags,
        };

        let Some((directory, _)) = wasi::filesystem::preopens::get_directories().into_iter().next()
        else {
            return;
        };
        let file = directory
            .open_at(
                PathFlags::empty(),
                "note.txt",
                OpenFlags::empty(),
                DescriptorFlags::READ | DescriptorFlags::WRITE,
            )
            .unwrap();
        drop(file.read_via_stream(0));
        drop(file.write_via_stream(0));
        drop(file.append_via_stream());
        let _ = file.advise(0, 0, Advice::Normal);
        let _ = file.sync_data();
        let _ = file.get_flags();
        let _ = file.get_type();
        let _ = file.set_size(4);
        let _ = file.set_times(NewTimestamp::NoChange, NewTimestamp::NoChange);
        let _ = file.read(4, 0);
        let _ = file.write(b"note", 0);
        let entries = directory.read_directory().unwrap();
        let _ = entries.read_directory_entry();
        drop(entries);
        let _ = file.sync();
        let _ = directory.create_directory_at("coverage-dir");
        let _ = directory.stat();
        let _ = directory.stat_at(PathFlags::empty(), "note.txt");
        let _ = directory.set_times_at(
            PathFlags::empty(),
            "note.txt",
            NewTimestamp::NoChange,
            NewTimestamp::NoChange,
        );
        let _ = directory.link_at(PathFlags::empty(), "note.txt", &directory, "hard-link");
        let _ = directory.symlink_at("note.txt", "symbolic-link");
        let _ = directory.readlink_at("symbolic-link");
        let _ = directory.rename_at("hard-link", &directory, "renamed-link");
        let _ = directory.unlink_file_at("renamed-link");
        let _ = directory.unlink_file_at("symbolic-link");
        let _ = directory.remove_directory_at("coverage-dir");
        let _ = directory.is_same_object(&directory);
        let _ = directory.metadata_hash();
        let _ = directory.metadata_hash_at(PathFlags::empty(), "note.txt");
    }

    fn directories() -> Vec<String> {
        wasi::filesystem::preopens::get_directories()
            .into_iter()
            .map(|(_, path)| path)
            .collect()
    }

    fn read_file(path: String) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|error| format!("{:?}", error.kind()))
    }

    fn write_file(path: String, contents: String) -> Result<(), String> {
        std::fs::write(path, contents).map_err(|error| format!("{:?}", error.kind()))
    }

    fn stat_preopen() -> Result<(), String> {
        let Some((directory, _)) = wasi::filesystem::preopens::get_directories().into_iter().next()
        else {
            return Err("no preopen".to_owned());
        };
        directory
            .stat()
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }
    fn link_preopens() {
        use wasi::filesystem::types::PathFlags;

        let mut directories = wasi::filesystem::preopens::get_directories().into_iter();
        let (source, _) = directories.next().unwrap();
        let (target, _) = directories.next().unwrap();
        let _ = source.link_at(PathFlags::empty(), "note.txt", &target, "linked.txt");
    }

    fn filesystem_channels() {
        use wasi::filesystem::types::{DescriptorFlags, OpenFlags, PathFlags};

        let (directory, _) = wasi::filesystem::preopens::get_directories()
            .into_iter()
            .next()
            .unwrap();
        let file = directory
            .open_at(
                PathFlags::empty(),
                "note.txt",
                OpenFlags::empty(),
                DescriptorFlags::READ | DescriptorFlags::WRITE,
            )
            .unwrap();
        drop(file.read_via_stream(0).unwrap());
        drop(file.write_via_stream(0).unwrap());
        drop(file.append_via_stream().unwrap());
    }

    fn descriptor_handle() -> bool {
        let (directory, _) = wasi::filesystem::preopens::get_directories()
            .into_iter()
            .next()
            .unwrap();
        matches!(
            directory.stat(),
            Err(wasi::filesystem::types::ErrorCode::Access)
        )
    }

    fn directory_stream_handle() -> bool {
        let (directory, _) = wasi::filesystem::preopens::get_directories()
            .into_iter()
            .next()
            .unwrap();
        let stream = directory.read_directory().unwrap();
        matches!(
            stream.read_directory_entry(),
            Err(wasi::filesystem::types::ErrorCode::Access)
        )
    }

    fn filesystem_differential() -> String {
        let before = std::fs::read_to_string("/data/note.txt").unwrap();
        std::fs::write("/data/output.txt", "written").unwrap();
        let after = std::fs::read_to_string("/data/output.txt").unwrap();
        format!("{before}|{after}")
    }

    fn sockets_denied() -> (String, String) {
        use wasi::sockets::network::IpAddressFamily;

        let tcp = wasi::sockets::tcp_create_socket::create_tcp_socket(IpAddressFamily::Ipv4)
            .expect_err("raw sockets should be disabled");
        let network = wasi::sockets::instance_network::instance_network();
        let dns = wasi::sockets::ip_name_lookup::resolve_addresses(&network, "localhost")
            .expect_err("name lookup should be disabled");
        (tcp.name().to_owned(), dns.name().to_owned())
    }

    fn tcp_echo(port: u16) -> Result<String, String> {
        tcp_echo(port)
    }

    fn udp_echo(port: u16) -> Result<String, String> {
        udp_echo(port)
    }

    fn socket_differential(tcp_port: u16, udp_port: u16) -> Result<String, String> {
        Ok(format!("{}|{}", tcp_echo(tcp_port)?, udp_echo(udp_port)?))
    }

    fn dns_probe(name: String) -> String {
        let network = wasi::sockets::instance_network::instance_network();
        match wasi::sockets::ip_name_lookup::resolve_addresses(&network, &name) {
            Ok(stream) => {
                drop(stream);
                "started".to_owned()
            }
            Err(error) => error.name().to_owned(),
        }
    }

    fn socket_handle() -> bool {
        use wasi::sockets::network::{ErrorCode, IpAddressFamily};

        let socket =
            wasi::sockets::tcp_create_socket::create_tcp_socket(IpAddressFamily::Ipv4).unwrap();
        matches!(socket.set_hop_limit(64), Err(ErrorCode::AccessDenied))
    }

    fn network_handle() -> bool {
        use wasi::sockets::network::ErrorCode;

        let network = wasi::sockets::instance_network::instance_network();
        matches!(
            wasi::sockets::ip_name_lookup::resolve_addresses(&network, "localhost"),
            Err(ErrorCode::AccessDenied)
        )
    }

    fn resolver_handle() -> bool {
        use wasi::sockets::network::ErrorCode;

        let network = wasi::sockets::instance_network::instance_network();
        let resolver =
            wasi::sockets::ip_name_lookup::resolve_addresses(&network, "localhost").unwrap();
        matches!(
            resolver.resolve_next_address(),
            Err(ErrorCode::AccessDenied)
        )
    }

    fn incoming_datagram_handle() -> bool {
        use wasi::sockets::network::ErrorCode;

        let (_socket, incoming, _outgoing) = datagram_streams();
        matches!(incoming.receive(1), Err(ErrorCode::AccessDenied))
    }

    fn outgoing_datagram_handle() -> bool {
        use wasi::sockets::network::ErrorCode;

        let (_socket, _incoming, outgoing) = datagram_streams();
        matches!(outgoing.check_send(), Err(ErrorCode::AccessDenied))
    }

    fn socket_coverage(tcp_port: u16, udp_port: u16) {
        use wasi::sockets::network::IpAddressFamily;

        let network = wasi::sockets::instance_network::instance_network();
        let lookup =
            wasi::sockets::ip_name_lookup::resolve_addresses(&network, "localhost").unwrap();
        drop(lookup.subscribe());
        let _ = lookup.resolve_next_address();

        let tcp = wasi::sockets::tcp_create_socket::create_tcp_socket(IpAddressFamily::Ipv4)
            .unwrap();
        let _ = tcp.address_family();
        let _ = tcp.is_listening();
        let _ = tcp.set_listen_backlog_size(1);
        let _ = tcp.keep_alive_enabled();
        let _ = tcp.set_keep_alive_enabled(true);
        let _ = tcp.keep_alive_idle_time();
        let _ = tcp.set_keep_alive_idle_time(1);
        let _ = tcp.keep_alive_interval();
        let _ = tcp.set_keep_alive_interval(1);
        let _ = tcp.keep_alive_count();
        let _ = tcp.set_keep_alive_count(1);
        let _ = tcp.hop_limit();
        let _ = tcp.set_hop_limit(64);
        let _ = tcp.receive_buffer_size();
        let _ = tcp.set_receive_buffer_size(4096);
        let _ = tcp.send_buffer_size();
        let _ = tcp.set_send_buffer_size(4096);
        drop(tcp.subscribe());
        tcp.start_connect(&network, socket_address(tcp_port))
            .unwrap();
        tcp.subscribe().block();
        let (input, output) = tcp.finish_connect().unwrap();
        let _ = tcp.local_address();
        let _ = tcp.remote_address();
        let _ = tcp.shutdown(wasi::sockets::tcp::ShutdownType::Both);
        drop(input);
        drop(output);

        let listener =
            wasi::sockets::tcp_create_socket::create_tcp_socket(IpAddressFamily::Ipv4).unwrap();
        listener
            .start_bind(&network, socket_address(0))
            .unwrap();
        listener.finish_bind().unwrap();
        listener.start_listen().unwrap();
        listener.finish_listen().unwrap();
        let _ = listener.accept();

        let udp = wasi::sockets::udp_create_socket::create_udp_socket(IpAddressFamily::Ipv4)
            .unwrap();
        let _ = udp.address_family();
        let _ = udp.unicast_hop_limit();
        let _ = udp.set_unicast_hop_limit(64);
        let _ = udp.receive_buffer_size();
        let _ = udp.set_receive_buffer_size(4096);
        let _ = udp.send_buffer_size();
        let _ = udp.set_send_buffer_size(4096);
        drop(udp.subscribe());
        udp.start_bind(&network, socket_address(0)).unwrap();
        udp.finish_bind().unwrap();
        let _ = udp.local_address();
        let _ = udp.remote_address();
        let (incoming, outgoing) = udp.stream(Some(socket_address(udp_port))).unwrap();
        drop(incoming.subscribe());
        let _ = incoming.receive(1);
        drop(outgoing.subscribe());
        let _ = outgoing.check_send();
        let _ = outgoing.send(&[wasi::sockets::udp::OutgoingDatagram {
            data: b"udp".to_vec(),
            remote_address: None,
        }]);
    }
}

fn datagram_streams() -> (
    wasi::sockets::udp::UdpSocket,
    wasi::sockets::udp::IncomingDatagramStream,
    wasi::sockets::udp::OutgoingDatagramStream,
) {
    use wasi::sockets::network::IpAddressFamily;

    let network = wasi::sockets::instance_network::instance_network();
    let socket =
        wasi::sockets::udp_create_socket::create_udp_socket(IpAddressFamily::Ipv4).unwrap();
    socket.start_bind(&network, socket_address(0)).unwrap();
    socket.finish_bind().unwrap();
    let (incoming, outgoing) = socket.stream(None).unwrap();
    (socket, incoming, outgoing)
}

fn socket_address(port: u16) -> wasi::sockets::network::IpSocketAddress {
    wasi::sockets::network::IpSocketAddress::Ipv4(
        wasi::sockets::network::Ipv4SocketAddress {
            port,
            address: (127, 0, 0, 1),
        },
    )
}

fn tcp_echo(port: u16) -> Result<String, String> {
    use std::io::{Read, Write};

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))
        .map_err(|error| format!("{:?}", error.kind()))?;
    stream
        .write_all(b"tcp")
        .map_err(|error| format!("{:?}", error.kind()))?;
    let mut reply = [0; 3];
    stream
        .read_exact(&mut reply)
        .map_err(|error| format!("{:?}", error.kind()))?;
    String::from_utf8(reply.to_vec()).map_err(|error| error.to_string())
}

fn udp_echo(port: u16) -> Result<String, String> {
    let socket = std::net::UdpSocket::bind(("127.0.0.1", 0))
        .map_err(|error| format!("{:?}", error.kind()))?;
    socket
        .connect(("127.0.0.1", port))
        .map_err(|error| format!("{:?}", error.kind()))?;
    socket
        .send(b"udp")
        .map_err(|error| format!("{:?}", error.kind()))?;
    let mut reply = [0; 3];
    socket
        .recv(&mut reply)
        .map_err(|error| format!("{:?}", error.kind()))?;
    String::from_utf8(reply.to_vec()).map_err(|error| error.to_string())
}

export!(Component);
