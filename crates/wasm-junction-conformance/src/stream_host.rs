use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction::{
    BoxFuture, Call, CallContext, CallError, InputStream, OutputStream, OutputStreamWriter,
    Provided, Provider, Val, Vals,
};

use crate::STREAM_HOST;

#[derive(Default)]
struct State {
    writer: Option<OutputStreamWriter>,
    audit: Vec<u8>,
    advanced: bool,
    reader_closed: bool,
    write_error: Option<String>,
}

/// The hand-written provider used by byte-stream conformance scenarios.
#[derive(Clone, Default)]
pub struct StreamHost(Arc<Mutex<State>>);

impl StreamHost {
    /// Wraps this host as the fixture's stream provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(STREAM_HOST, self)
    }

    /// Returns the bytes received by the latest `audit` call.
    #[must_use]
    pub fn audit(&self) -> Vec<u8> {
        self.lock().audit.clone()
    }

    /// Reports whether the guest requested the second incremental chunk.
    #[must_use]
    pub fn advanced(&self) -> bool {
        self.lock().advanced
    }

    /// Reports whether a write observed that the guest abandoned its reader.
    #[must_use]
    pub fn reader_closed(&self) -> bool {
        self.lock().reader_closed
    }

    /// Returns the latest incremental write failure.
    #[must_use]
    pub fn write_error(&self) -> Option<String> {
        self.lock().write_error.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Provider for StreamHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            match call.function.as_ref() {
                "motd" => Ok(vec![OutputStream::from_bytes(b"Have a good day.").into()]),
                "audit" => {
                    let [value] = <[_; 1]>::try_from(call.args)
                        .map_err(|_| CallError::trap("audit expects one stream"))?;
                    let input = InputStream::try_from(value)?;
                    self.lock().audit = input
                        .read_all()
                        .await
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(Vec::new())
                }
                "optional" => {
                    let [Val::Option(Some(value))] = <[_; 1]>::try_from(call.args)
                        .map_err(|_| CallError::trap("optional expects one stream option"))?
                    else {
                        return Err(CallError::trap("optional expects some stream"));
                    };
                    let bytes = InputStream::try_from(*value)?
                        .read_all()
                        .await
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(vec![Val::Option(Some(Box::new(
                        OutputStream::from_bytes(bytes).into(),
                    )))])
                }
                "chunks" => {
                    let (writer, stream) = OutputStream::channel();
                    writer
                        .write(b"first ")
                        .await
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    self.lock().writer = Some(writer);
                    Ok(vec![stream.into()])
                }
                "advance" => {
                    let writer = self
                        .lock()
                        .writer
                        .take()
                        .ok_or_else(|| CallError::trap("no incremental stream"))?;
                    let error = writer.write(b"second").await.err();
                    let mut state = self.lock();
                    state.advanced = true;
                    state.reader_closed = error.is_some();
                    state.write_error = error.map(|error| error.to_string());
                    Ok(Vec::new())
                }
                function => Err(CallError::unavailable(format!(
                    "stream host has no `{function}` function"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use wasm_junction::{Caller, StreamHandle};

    use super::*;

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("future unexpectedly suspended"),
        }
    }

    fn call(function: &str, args: Vals) -> Call {
        Call::new(Caller::Host, "host", STREAM_HOST, function, args)
    }

    #[test]
    fn host_produces_incrementally_and_consumes_audit_bytes() {
        let host = StreamHost::default();
        let context = CallContext::for_test("streams");
        let mut values = ready(host.call(&context, call("chunks", Vec::new()))).unwrap();
        let Val::Stream(handle) = values.remove(0) else {
            panic!("chunks did not return a stream");
        };
        let mut input = InputStream::try_from(handle).unwrap();
        assert_eq!(ready(input.read()).unwrap(), Some(b"first ".to_vec()));
        ready(host.call(&context, call("advance", Vec::new()))).unwrap();
        assert_eq!(ready(input.read_all()).unwrap(), b"second");

        let audit = Val::Stream(StreamHandle::from(OutputStream::from_bytes(b"entry")));
        ready(host.call(&context, call("audit", vec![audit]))).unwrap();
        assert_eq!(host.audit(), b"entry");
    }
}
