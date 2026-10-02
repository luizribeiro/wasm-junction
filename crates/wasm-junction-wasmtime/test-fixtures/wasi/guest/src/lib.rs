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
        format!(
            "{environment:?}|{arguments:?}|{cwd:?}|{}:{}|{monotonic}|{}|{}|{timers_ordered}",
            wall.seconds,
            wall.nanoseconds,
            wall_now.seconds > 0,
            second >= first
        )
    }
}

export!(Component);
