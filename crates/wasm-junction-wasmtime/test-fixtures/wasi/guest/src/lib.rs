wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
});

struct Component;

impl exports::test::wasi::environment::Guest for Component {
    fn read(name: String) -> Option<String> {
        std::env::var(name).ok()
    }
}

export!(Component);
