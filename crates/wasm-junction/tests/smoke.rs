//! Native linkage smoke test.

use wasm_junction as _;

#[test]
fn links_without_an_engine() {
    assert_eq!(env!("CARGO_PKG_NAME"), "wasm-junction");
}
