//! Exact-output check for the filesystem executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn files_prints_gated_calls_and_policy_results() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-files"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
