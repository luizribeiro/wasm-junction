//! Exact-output check for the reload executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn reload_prints_pinned_and_replaced_calls() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-reload"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
