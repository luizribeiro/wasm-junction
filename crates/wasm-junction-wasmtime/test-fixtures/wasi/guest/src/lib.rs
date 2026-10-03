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
}

export!(Component);
