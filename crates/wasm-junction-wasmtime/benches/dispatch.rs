//! Five-sample release measurements for the full application dispatcher.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[path = "../benchmark/support.rs"]
mod support;

use std::future::Future;
use std::time::{Duration, Instant};

use support::{Counting, block_on, imports, loaded, noop};

const CALLS: u32 = 100_000;
const INSTANTIATIONS: u32 = 1_000;
const SAMPLES: usize = 5;

fn main() {
    let empty = block_on(loaded(None));
    let counting = Counting::default();
    let with_middleware = block_on(loaded(Some(counting.clone())));

    block_on(imports(&empty, 1));
    block_on(imports(&with_middleware, 1));
    block_on(noop(&empty));

    let empty_samples = collect_samples(|| imports(&empty, CALLS));
    let middleware_samples = collect_samples(|| imports(&with_middleware, CALLS));
    assert!(counting.calls() >= u64::from(CALLS) * 5);

    let instantiation_samples = collect_samples(|| async {
        for _ in 0..INSTANTIATIONS {
            noop(&empty).await;
        }
    });

    print_ns("Val import, empty middleware", &empty_samples, CALLS);
    print_ns(
        "Val import, one counting middleware",
        &middleware_samples,
        CALLS,
    );
    print_us(
        "fresh instance + noop export",
        &instantiation_samples,
        INSTANTIATIONS,
    );
}

fn collect_samples<F, Fut>(mut run: F) -> Vec<Duration>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            block_on(run());
            start.elapsed()
        })
        .collect()
}

fn median(samples: &[Duration]) -> Duration {
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    ordered[ordered.len() / 2]
}

fn print_ns(label: &str, samples: &[Duration], operations: u32) {
    let per_operation =
        |sample: &Duration| sample.as_secs_f64() * 1_000_000_000.0 / f64::from(operations);
    let values = samples.iter().map(per_operation).collect::<Vec<_>>();
    println!(
        "{label}: {:.1} ns/call; samples {values:.1?}",
        per_operation(&median(samples))
    );
}

fn print_us(label: &str, samples: &[Duration], operations: u32) {
    let per_operation =
        |sample: &Duration| sample.as_secs_f64() * 1_000_000.0 / f64::from(operations);
    let values = samples.iter().map(per_operation).collect::<Vec<_>>();
    println!(
        "{label}: {:.1} us/call; samples {values:.1?}",
        per_operation(&median(samples))
    );
}
