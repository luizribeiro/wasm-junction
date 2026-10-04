wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
    generate_all,
});

use std::future::{Future, IntoFuture, poll_fn};
use std::task::Poll;

struct Component;

async fn join<A: Future, B: Future>(left: A, right: B) -> (A::Output, B::Output) {
    let mut left = std::pin::pin!(left);
    let mut right = std::pin::pin!(right);
    let mut left_result = None;
    let mut right_result = None;
    poll_fn(move |context| {
        if left_result.is_none()
            && let Poll::Ready(result) = left.as_mut().poll(context)
        {
            left_result = Some(result);
        }
        if right_result.is_none()
            && let Poll::Ready(result) = right.as_mut().poll(context)
        {
            right_result = Some(result);
        }
        match (left_result.take(), right_result.take()) {
            (Some(left), Some(right)) => Poll::Ready((left, right)),
            (left, right) => {
                left_result = left;
                right_result = right;
                Poll::Pending
            }
        }
    })
    .await
}

async fn write_stdout(bytes: Vec<u8>) -> bool {
    let (mut writer, reader) = wit_stream::new();
    let write = async move {
        assert!(writer.write_all(bytes).await.is_empty());
    };
    let completion = wasi::cli::stdout::write_via_stream(reader).into_future();
    let (_, result) = join(write, completion).await;
    result.is_ok()
}

async fn write_stderr(bytes: Vec<u8>) -> bool {
    let (mut writer, reader) = wit_stream::new();
    let write = async move {
        assert!(writer.write_all(bytes).await.is_empty());
    };
    let completion = wasi::cli::stderr::write_via_stream(reader).into_future();
    let (_, result) = join(write, completion).await;
    result.is_ok()
}

async fn cli_probe() -> String {
    let environment = wasi::cli::environment::get_environment();
    let arguments = wasi::cli::environment::get_arguments();
    let cwd = wasi::cli::environment::get_initial_cwd();
    let (input, completion) = wasi::cli::stdin::read_via_stream();
    let input = input.collect().await;
    let input_result = match completion.await {
        Ok(()) => "ok",
        Err(wasi::cli::types::ErrorCode::Io) => "io",
        Err(wasi::cli::types::ErrorCode::IllegalByteSequence) => "illegal-byte-sequence",
        Err(wasi::cli::types::ErrorCode::Pipe) => "pipe",
    };
    let stdout_ok = write_stdout(b"stdout".to_vec()).await;
    let stderr_ok = write_stderr(b"stderr".to_vec()).await;
    let terminals = (
        wasi::cli::terminal_stdin::get_terminal_stdin().is_some(),
        wasi::cli::terminal_stdout::get_terminal_stdout().is_some(),
        wasi::cli::terminal_stderr::get_terminal_stderr().is_some(),
    );
    format!(
        "{environment:?}|{arguments:?}|{cwd:?}|{}|{input_result}|{stdout_ok}|{stderr_ok}|{terminals:?}",
        input.len()
    )
}

impl exports::test::wasi_p3::probe::Guest for Component {
    async fn coverage() -> String {
        let _ = cli_probe().await;
        let resolution = wasi::clocks::monotonic_clock::get_resolution();
        let first = wasi::clocks::monotonic_clock::now();
        wasi::clocks::monotonic_clock::wait_for(1_000_000).await;
        let second = wasi::clocks::monotonic_clock::now();
        wasi::clocks::monotonic_clock::wait_until(second).await;
        let system = wasi::clocks::system_clock::now();
        let system_resolution = wasi::clocks::system_clock::get_resolution();
        let random = wasi::random::random::get_random_bytes(4);
        let insecure = wasi::random::insecure::get_insecure_random_bytes(5);
        let _ = wasi::random::random::get_random_u64();
        let _ = wasi::random::insecure::get_insecure_random_u64();
        let _ = wasi::random::insecure_seed::get_insecure_seed();
        format!(
            "{}|{}|{}|{}|{}|{}",
            second >= first,
            system.seconds > 0,
            random.len(),
            insecure.len(),
            resolution > 0,
            system_resolution > 0,
        )
    }

    async fn cli() -> String {
        cli_probe().await
    }

    async fn exit_success() {
        wasi::cli::exit::exit(Ok(()));
    }

    async fn exit_code() {
        wasi::cli::exit::exit_with_code(7);
    }
}

export!(Component);
