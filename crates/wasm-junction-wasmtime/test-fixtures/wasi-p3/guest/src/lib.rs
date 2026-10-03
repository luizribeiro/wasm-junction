wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
    generate_all,
});

struct Component;

impl exports::test::wasi_p3::probe::Guest for Component {
    async fn coverage() -> String {
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
}

export!(Component);
