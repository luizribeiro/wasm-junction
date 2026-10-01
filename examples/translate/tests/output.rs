//! Exact-output check for the translate executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn translate_prints_routes_refusal_and_results() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-translate"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
