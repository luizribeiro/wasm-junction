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
}

export!(Component);
