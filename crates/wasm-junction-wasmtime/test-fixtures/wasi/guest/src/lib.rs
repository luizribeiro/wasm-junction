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
        format!(
            "{environment:?}|{arguments:?}|{cwd:?}|{}:{}|{monotonic}|{}|{}|{timers_ordered}|{stream}|{}:{}",
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
}

export!(Component);
