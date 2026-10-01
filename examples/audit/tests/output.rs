//! Exact-output check for the audit executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn audit_prints_the_complete_trace_and_tagged_lines() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-audit"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
