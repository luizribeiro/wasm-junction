//! Exact-output check for the HTTP executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn http_prints_policy_decisions_and_results() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-http"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
