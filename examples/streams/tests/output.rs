//! Exact-output check for the streams executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::Command;

#[test]
fn streams_prints_the_redacted_transcript_and_visible_tickets() {
    let output = Command::new(env!("CARGO_BIN_EXE_wasm-junction-example-streams"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("expected-output.txt")
    );
}
