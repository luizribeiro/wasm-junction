use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction::{CallContext, CallError, InputStream, OutputStream, Provided};

use crate::value_stream_bindings::host;

#[derive(Default)]
struct State {
    strings: Vec<String>,
    lists: Vec<Vec<u32>>,
}

/// The host used by value-stream conformance scenarios.
#[derive(Clone, Default)]
pub struct ValueStreamHost(Arc<Mutex<State>>);

impl ValueStreamHost {
    /// Wraps this host as the fixture's value-stream provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        host::provider(self)
    }

    /// Returns the string items received from the guest.
    #[must_use]
    pub fn strings(&self) -> Vec<String> {
        self.lock().strings.clone()
    }

    /// Returns the list items received from the guest.
    #[must_use]
    pub fn lists(&self) -> Vec<Vec<u32>> {
        self.lock().lists.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl host::Host for ValueStreamHost {
    async fn accept_strings(
        &self,
        _cx: &CallContext,
        values: InputStream<String>,
    ) -> Result<(), CallError> {
        self.lock().strings = values.read_all().await?;
        Ok(())
    }

    async fn accept_lists(
        &self,
        _cx: &CallContext,
        values: InputStream<Vec<u32>>,
    ) -> Result<(), CallError> {
        self.lock().lists = values.read_all().await?;
        Ok(())
    }

    fn strings(&self, _cx: &CallContext) -> Result<OutputStream<String>, CallError> {
        Ok(OutputStream::from_items([
            "host one".to_owned(),
            "host two".to_owned(),
        ]))
    }
}
