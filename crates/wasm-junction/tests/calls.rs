//! Call representation and typed-view tests.

mod support;

use support::read_call;
use wasm_junction::{Trap, TypeError};

#[test]
fn clone_preserves_call_metadata_and_arguments() {
    let call = read_call("daily");
    assert_eq!(call.clone(), call);
    assert_eq!(
        call.to_string(),
        "summarizer → notebook example:journal/notes@0.1.0.read"
    );
}

#[test]
fn type_errors_become_boundary_traps() {
    let trap = Trap::from(TypeError::new("wrong argument shape"));
    assert_eq!(trap.to_string(), "wrong argument shape");
}
