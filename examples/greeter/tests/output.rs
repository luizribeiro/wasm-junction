//! Exact-output check for the greeter executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn greeter_prints_calls_returns_and_results() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-greeter"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
