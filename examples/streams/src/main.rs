//! Native entry point for the streams example.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::error::Error;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    wasm_junction_example_streams::run(|line| println!("{line}")).await
}
