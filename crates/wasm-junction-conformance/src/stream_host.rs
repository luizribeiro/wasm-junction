use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use wasm_junction::{
    BoxFuture, Call, CallContext, CallError, InputStream, OutputStream, OutputStreamWriter,
    Provided, Provider, Vals,
};

use crate::stream_bindings::host;

#[derive(Default)]
struct State {
    writer: Option<OutputStreamWriter>,
    audit: Vec<u8>,
    advanced: bool,
    reader_closed: bool,
    write_error: Option<String>,
    value_calls: usize,
}

/// The host used by byte-stream conformance scenarios.
#[derive(Clone, Default)]
pub struct StreamHost(Arc<Mutex<State>>);

#[derive(Default)]
struct RetainedState {
    first: Vec<u8>,
    input: Option<InputStream>,
    checkpoint: Option<Waker>,
    first_checkpoint: bool,
    second_ready: bool,
    audit: Option<Waker>,
}

/// A host that retains a guest stream beyond its imported call.
#[derive(Clone, Default)]
pub struct RetainHost(Arc<Mutex<RetainedState>>);

#[derive(Default)]
struct PoisonState {
    source: Option<OutputStreamWriter>,
    guest: Option<InputStream>,
    advances: usize,
}

/// A host that refuses one import and records any later stream effects.
#[derive(Clone, Default)]
pub struct PoisonHost(Arc<Mutex<PoisonState>>);

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

    /// Returns how many unsupported value streams reached the host implementation.
    #[must_use]
    pub fn value_calls(&self) -> usize {
        self.lock().value_calls
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl RetainHost {
    /// Wraps this host as the fixture's stream provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(crate::STREAM_HOST, self)
    }

    /// Returns the first chunk read before the guest reaches its checkpoint.
    #[must_use]
    pub fn first(&self) -> Vec<u8> {
        self.lock().first.clone()
    }

    /// Takes the retained stream reader after the fixture call completes.
    #[must_use]
    pub fn take_input(&self) -> Option<InputStream> {
        self.lock().input.take()
    }

    fn lock(&self) -> MutexGuard<'_, RetainedState> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl PoisonHost {
    /// Wraps this host as the fixture's stream provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(crate::STREAM_HOST, self)
    }

    /// Takes the guest stream retained by the refused import.
    #[must_use]
    pub fn take_guest(&self) -> Option<InputStream> {
        self.lock().guest.take()
    }

    /// Takes the writer for the unread host stream.
    #[must_use]
    pub fn take_source(&self) -> Option<OutputStreamWriter> {
        self.lock().source.take()
    }

    /// Returns how many later `advance` calls reached the host.
    #[must_use]
    pub fn advances(&self) -> usize {
        self.lock().advances
    }

    fn lock(&self) -> MutexGuard<'_, PoisonState> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Provider for PoisonHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            match call.function.as_ref() {
                "chunks" => {
                    let (writer, stream) = OutputStream::channel();
                    writer.write(b"unread").await?;
                    self.lock().source = Some(writer);
                    Ok(vec![stream.into()])
                }
                "poison" => {
                    let [value] = <[_; 1]>::try_from(call.args)
                        .map_err(|_| CallError::trap("poison expects one stream"))?;
                    self.lock().guest = Some(InputStream::try_from(value)?);
                    Err(CallError::refused("stream refused"))
                }
                "advance" => {
                    self.lock().advances += 1;
                    Ok(Vec::new())
                }
                function => Err(CallError::unavailable(format!(
                    "poison host has no `{function}` function"
                ))),
            }
        })
    }
}

impl Provider for RetainHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            if call.function.as_ref() == "checkpoint" {
                poll_fn(|context| {
                    let mut state = self.lock();
                    if state.first.is_empty() {
                        state.checkpoint = Some(context.waker().clone());
                        Poll::Pending
                    } else if !state.first_checkpoint {
                        state.first_checkpoint = true;
                        Poll::Ready(())
                    } else {
                        state.second_ready = true;
                        if let Some(audit) = state.audit.take() {
                            audit.wake();
                        }
                        Poll::Ready(())
                    }
                })
                .await;
                return Ok(Vec::new());
            }
            let [value] = <[_; 1]>::try_from(call.args)
                .map_err(|_| CallError::trap("audit expects one stream"))?;
            let mut input = InputStream::try_from(value)?;
            let first = input
                .read()
                .await
                .map_err(|error| CallError::trap(error.to_string()))?
                .ok_or_else(|| CallError::trap("guest stream closed before its first chunk"))?;
            let checkpoint = {
                let mut state = self.lock();
                state.first = first;
                state.input = Some(input);
                state.checkpoint.take()
            };
            if let Some(checkpoint) = checkpoint {
                checkpoint.wake();
            }
            poll_fn(|context| {
                let mut state = self.lock();
                if state.second_ready {
                    Poll::Ready(())
                } else {
                    state.audit = Some(context.waker().clone());
                    Poll::Pending
                }
            })
            .await;
            Ok(Vec::new())
        })
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

    fn poison(
        &self,
        _cx: &CallContext,
        _lines: InputStream,
    ) -> impl Future<Output = Result<(), CallError>> {
        std::future::ready(Err(CallError::refused("stream refused")))
    }

    fn accept_values(
        &self,
        _cx: &CallContext,
        _values: InputStream<host::Note>,
    ) -> impl Future<Output = Result<(), CallError>> {
        self.lock().value_calls += 1;
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
        let (writer, output) = OutputStream::<u8>::channel();
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
