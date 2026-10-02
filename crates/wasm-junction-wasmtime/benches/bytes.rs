//! Five-sample release measurements for a 64 KiB byte-list app call.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[path = "../benchmark/support.rs"]
#[allow(dead_code, reason = "shared support serves the dispatch benchmarks")]
mod support;

use std::hint::black_box;
use std::time::{Duration, Instant};

use support::{block_on, loaded};
use wasm_junction::Val;
use wasm_junction_conformance::DISPATCH_RUNNER;

const BYTES: usize = 64 * 1024;
const CALLS: u32 = 1_000;
const SAMPLES: usize = 5;

fn main() {
    let app = block_on(loaded(None));
    let bytes = vec![42; BYTES];
    block_on(echo(&app, &bytes));

    let samples = (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..CALLS {
                black_box(block_on(echo(&app, &bytes)));
            }
            start.elapsed()
        })
        .collect::<Vec<_>>();
    print(&samples);
}

async fn echo(app: &wasm_junction::App, bytes: &[u8]) -> Vec<u8> {
    let values = app
        .call(
            "dispatch",
            DISPATCH_RUNNER,
            "echo-bytes",
            vec![Val::Bytes(bytes.to_vec())],
        )
        .await
        .unwrap();
    let [Val::Bytes(bytes)] = <[Val; 1]>::try_from(values).unwrap() else {
        panic!("echo returned the wrong value shape")
    };
    bytes
}

fn print(samples: &[Duration]) {
    let per_call = |sample: &Duration| sample.as_secs_f64() * 1_000_000.0 / f64::from(CALLS);
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let values = samples.iter().map(per_call).collect::<Vec<_>>();
    println!(
        "64 KiB list<u8> app echo: {:.1} us/call; samples {values:.1?}",
        per_call(&ordered[SAMPLES / 2])
    );
}
