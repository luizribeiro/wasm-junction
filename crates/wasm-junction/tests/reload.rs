//! Component lifecycle tests using an engine-neutral fake engine.

mod support;

use std::sync::Arc;

use support::{GenerationEngine, GenerationState, block_on, component_bytes};
use wasm_junction::{App, Component, Val};

const TRANSLATOR: &str = "example:translate/translator@1.0.0";
const TRANSLATOR_WIT: &str = r"
    package example:translate@1.0.0;
    interface translator { translate: func(text: string) -> string; }
    world service { export translator; }
";

fn component() -> Component {
    Component::from_bytes(component_bytes(TRANSLATOR_WIT, "service")).unwrap()
}

#[test]
fn in_flight_calls_pin_the_old_generation_until_they_finish() {
    let state = Arc::new(GenerationState::default());
    let app = App::builder()
        .engine(GenerationEngine(state.clone()))
        .build()
        .unwrap();
    block_on(app.load(component().named("translator"))).unwrap();

    let caller = app.clone();
    let in_flight = std::thread::spawn(move || {
        block_on(caller.call("translator", TRANSLATOR, "translate", Vec::new())).unwrap()
    });
    state.wait_until_called();
    block_on(app.reload("translator", component())).unwrap();
    let old = state.generation(0);
    assert!(old.upgrade().is_some());
    state.release();

    assert_eq!(in_flight.join().unwrap(), [Val::from("old")]);
    assert!(old.upgrade().is_none());
    assert_eq!(
        block_on(app.call("translator", TRANSLATOR, "translate", Vec::new())).unwrap(),
        [Val::from("new")]
    );
}
