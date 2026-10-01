//! Controllable host import used to suspend one guest call.

use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};

use wasm_junction::{CallContext, CallError, Provided};

use crate::bindings::gate;

#[derive(Default)]
struct State {
    entered: bool,
    released: bool,
    waker: Option<Waker>,
}

/// Gate shared by the host and the guest's slow greeting.
#[derive(Clone, Default)]
pub struct Gate(Arc<Mutex<State>>);

impl Gate {
    /// Wraps the gate as a host provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        gate::provider(self)
    }

    /// Reports whether the slow guest call reached the gate.
    #[must_use]
    pub fn entered(&self) -> bool {
        self.lock().entered
    }

    /// Releases the slow guest call.
    pub fn release(&self) {
        let mut state = self.lock();
        state.released = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl gate::Host for Gate {
    fn wait(
        &self,
        _context: &CallContext,
    ) -> impl std::future::Future<Output = Result<(), CallError>> {
        poll_fn(|context| {
            let mut state = self.lock();
            state.entered = true;
            if state.released {
                Poll::Ready(Ok(()))
            } else {
                state.waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
    }
}
