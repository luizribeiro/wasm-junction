use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use wasm_junction::{
    CallContext, CallError, InputStream, OutputStream, OutputStreamWriter, Provided,
};

use crate::stream_bindings::host;

#[derive(Default)]
struct State {
    writer: Option<OutputStreamWriter>,
    audit: Vec<u8>,
    advanced: bool,
    reader_closed: bool,
    write_error: Option<String>,
}

/// The host used by byte-stream conformance scenarios.
#[derive(Clone, Default)]
pub struct StreamHost(Arc<Mutex<State>>);

impl StreamHost {
    /// Wraps this host as the fixture's stream provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        host::provider(self)
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

impl host::Host for StreamHost {
    fn motd(&self, _cx: &CallContext) -> Result<OutputStream, CallError> {
        Ok(OutputStream::from_bytes(b"Have a good day."))
    }

    async fn audit(&self, _cx: &CallContext, lines: InputStream) -> Result<(), CallError> {
        self.lock().audit = lines.read_all().await?;
        Ok(())
    }

    async fn optional(
        &self,
        _cx: &CallContext,
        bytes: Option<InputStream>,
    ) -> Result<Option<OutputStream>, CallError> {
        let Some(bytes) = bytes else { return Ok(None) };
        Ok(Some(OutputStream::from_bytes(bytes.read_all().await?)))
    }

    fn chunks(&self, _cx: &CallContext) -> Result<OutputStream, CallError> {
        let (writer, stream) = OutputStream::channel();
        if ready(writer.write(b"first ")).is_ok() {
            self.lock().writer = Some(writer);
        }
        Ok(stream)
    }

    fn advance(&self, _cx: &CallContext) -> Result<(), CallError> {
        let Some(writer) = self.lock().writer.take() else {
            return Ok(());
        };
        let error = ready(writer.write(b"second")).err();
        let mut state = self.lock();
        state.advanced = true;
        state.reader_closed = error.is_some();
        state.write_error = error.map(|error| error.to_string());
        Ok(())
    }

    fn checkpoint(&self, _cx: &CallContext) -> impl Future<Output = Result<(), CallError>> {
        std::future::ready(Ok(()))
    }
}

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

#[cfg(test)]
mod tests {
    use wasm_junction::{CallErrorKind, StreamHandle};

    use super::*;

    #[test]
    fn host_produces_incrementally_and_consumes_audit_bytes() {
        let host = StreamHost::default();
        let context = CallContext::for_test("streams");
        let stream = host::Host::chunks(&host, &context).unwrap();
        let mut input = InputStream::try_from(StreamHandle::from(stream)).unwrap();
        assert_eq!(ready(input.read()).unwrap(), Some(b"first ".to_vec()));
        host::Host::advance(&host, &context).unwrap();
        assert_eq!(ready(input.read_all()).unwrap(), b"second");

        let audit =
            InputStream::try_from(StreamHandle::from(OutputStream::from_bytes(b"entry"))).unwrap();
        ready(host::Host::audit(&host, &context, audit)).unwrap();
        assert_eq!(host.audit(), b"entry");
    }

    #[test]
    fn failed_read_fails_the_host_call() {
        let host = StreamHost::default();
        let context = CallContext::for_test("streams");
        let (writer, output) = OutputStream::channel();
        let input = InputStream::try_from(StreamHandle::from(output)).unwrap();
        writer.abort();

        let error = ready(host::Host::audit(&host, &context, input)).unwrap_err();
        assert_eq!(error.kind(), CallErrorKind::Trap);
        assert_eq!(
            error.to_string(),
            "stream was aborted when its invocation ended"
        );
    }
}
