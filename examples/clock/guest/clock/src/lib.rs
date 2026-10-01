wit_bindgen::generate!({
    path: "../../wit",
    world: "plugin",
});

use std::time::{SystemTime, UNIX_EPOCH};

struct Component;

impl exports::example::clock::clock::Guest for Component {
    fn run() -> String {
        let greeting = std::env::var("GREETING").unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!(
            "{greeting} at {}.{:09}",
            now.as_secs(),
            now.subsec_nanos()
        )
    }
}

export!(Component);
