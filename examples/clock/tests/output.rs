//! Exact-output check for the clock executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn clock_prints_gated_calls_and_fixed_time() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-clock"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
