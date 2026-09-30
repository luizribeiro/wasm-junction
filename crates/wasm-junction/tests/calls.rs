//! Call representation and typed-view tests.

mod support;

use support::{Delete, Read, read_call};
use wasm_junction::{Trap, TypeError, TypedCall, Val};

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

#[test]
fn typed_arguments_encode_and_decode() {
    let call = read_call("daily");
    let read = Read::from_vals(&call.args).unwrap();
    assert_eq!(read.name, "daily");
    assert_eq!(read.into_vals(), vec![Val::from("daily")]);
}

#[test]
fn typed_outputs_encode_and_decode() {
    let values = Read::output(String::from("buy tea"));
    assert_eq!(Read::decode_output(&values).unwrap(), "buy tea");
    assert!(Read::decode_output(&[]).is_err());
    assert!(Read::decode_output(&[Val::U32(3)]).is_err());
    Delete::decode_output(&Delete::output(())).unwrap();
}
