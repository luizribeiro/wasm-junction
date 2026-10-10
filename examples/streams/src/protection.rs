//! Stream transformations used by this example.

use std::sync::{Arc, Mutex};

use wasm_junction::{Call, CallError, Middleware, Next, Val, Vals};

const EMAIL: &[u8] = b"ada@example.com";
const REDACTED_EMAIL: &[u8] = b"[redacted email]";
const SUPPORT_INTERFACE: &str = "example:streams/support@0.1.0";

pub(super) struct ProtectTickets;

impl Middleware for ProtectTickets {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        let stores_transcript = call.interface.as_ref() == SUPPORT_INTERFACE
            && call.function.as_ref() == "store-transcript";
        if stores_transcript && let Some(Val::Stream(stream)) = call.args.first_mut() {
            let redactor = Arc::new(Mutex::new(EmailRedactor::default()));
            let flushed = redactor.clone();
            *stream = stream.take().map_chunks_with_flush(
                move |chunk| {
                    redactor
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .redact(chunk)
                },
                move || {
                    flushed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .finish()
                },
            );
        }

        let filters_tickets =
            call.interface.as_ref() == SUPPORT_INTERFACE && call.function.as_ref() == "tickets";
        let mut result = next.run(call).await?;
        if filters_tickets && let Some(Val::Stream(stream)) = result.first_mut() {
            *stream = stream.take().filter_items(
                |item| !matches!(item, Val::String(ticket) if ticket.starts_with("private: ")),
            );
        }
        Ok(result)
    }
}

#[derive(Default)]
struct EmailRedactor {
    pending: Vec<u8>,
}

impl EmailRedactor {
    fn redact(&mut self, chunk: Vec<u8>) -> Vec<u8> {
        self.pending.extend(chunk);
        let mut output = Vec::new();
        loop {
            if self.pending.starts_with(EMAIL) {
                output.extend_from_slice(REDACTED_EMAIL);
                self.pending.drain(..EMAIL.len());
            } else if EMAIL.starts_with(&self.pending) {
                break;
            } else {
                output.push(self.pending.remove(0));
            }
        }
        output
    }

    fn finish(&mut self) -> Vec<u8> {
        // `redact` consumes complete matches, so only a non-email prefix can remain.
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::EmailRedactor;

    #[test]
    fn redacts_an_email_at_every_chunk_boundary() {
        let transcript = b"Customer email: ada@example.com\nIssue open\n";
        for split in 0..transcript.len() {
            let mut redactor = EmailRedactor::default();
            let (first, second) = transcript.split_at(split);
            let mut output = redactor.redact(first.to_vec());
            output.extend(redactor.redact(second.to_vec()));
            assert_eq!(
                output, b"Customer email: [redacted email]\nIssue open\n",
                "split offset {split}"
            );
        }
    }

    #[test]
    fn flushes_a_partial_candidate_at_end_of_stream() {
        let mut redactor = EmailRedactor::default();
        let mut output = redactor.redact(b"Customer email: ada@exa".to_vec());
        output.extend(redactor.finish());
        assert_eq!(output, b"Customer email: ada@exa");
        assert!(redactor.pending.is_empty());
    }

    #[test]
    fn redacts_a_complete_email_at_end_of_stream() {
        let mut redactor = EmailRedactor::default();
        let mut output = redactor.redact(b"Customer email: ada@example.com".to_vec());
        output.extend(redactor.finish());
        assert_eq!(output, b"Customer email: [redacted email]");
        assert!(redactor.pending.is_empty());
    }
}
