wit_bindgen::generate!({
    path: "../../wit",
    world: "plugin",
});

struct Component;

impl exports::example::files::files::Guest for Component {
    fn read(path: String) -> String {
        match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) => format!("{:?}", error.kind()),
        }
    }
}

export!(Component);
