use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};

use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, Handle, InterfaceHandle, Provided, Provider, Val,
    Vals,
};

use crate::RELOAD_GATE;

#[derive(Default)]
struct GateState {
    entered: bool,
    released: bool,
    waiting: bool,
    waker: Option<Waker>,
    entered_waker: Option<Waker>,
}

/// A host import that can suspend a reload fixture call.
#[derive(Clone, Default)]
pub struct ReloadHost(Arc<Mutex<GateState>>);

impl ReloadHost {
    /// Wraps this host as the fixture's gate provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(RELOAD_GATE, self)
    }

    /// Reports whether a guest is waiting at the gate.
    #[must_use]
    pub fn entered(&self) -> bool {
        self.lock().entered
    }

    /// Waits until a guest call has entered the gate provider.
    pub async fn wait_until_entered(&self) {
        poll_fn(|context| {
            let mut state = self.lock();
            if state.entered {
                Poll::Ready(())
            } else {
                state.entered_waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
        .await;
    }

    /// Prepares the gate for another suspended call.
    ///
    /// # Errors
    ///
    /// Returns an error if the previous guest call is still waiting at the gate.
    pub fn reset(&self) -> Result<(), CallError> {
        let mut state = self.lock();
        if state.waiting {
            return Err(CallError::refused(
                "cannot reset the reload gate while a call is waiting",
            ));
        }
        *state = GateState::default();
        Ok(())
    }

    /// Lets a suspended guest call continue.
    pub fn release(&self) {
        let mut state = self.lock();
        state.released = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Provider for ReloadHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            if call.function.as_ref() != "wait" {
                return Err(CallError::unavailable("reload gate has no such function"));
            }
            poll_fn(|context| {
                let (released, entered_waker) = {
                    let mut state = self.lock();
                    state.entered = true;
                    state.waiting = true;
                    state.waker = Some(context.waker().clone());
                    (state.released, state.entered_waker.take())
                };
                if let Some(waker) = entered_waker {
                    waker.wake();
                }
                if released {
                    let mut state = self.lock();
                    state.waiting = false;
                    state.waker = None;
                    Poll::Ready(Ok(Vec::new()))
                } else {
                    Poll::Pending
                }
            })
            .await
        })
    }
}

/// A typed handle to the reload fixture's greeter interface.
pub struct ReloadGreeter(Handle);

impl InterfaceHandle for ReloadGreeter {
    const INTERFACE: &'static str = crate::RELOAD_GREETER;

    fn from_app(app: App, component: Arc<str>) -> Self {
        Self(Handle::new(app, component, Self::INTERFACE))
    }
}

impl ReloadGreeter {
    /// Calls the slow greeting that waits on [`ReloadHost`].
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when dispatch, the guest, or its host import fails.
    pub async fn greet_slow(&self, name: &str) -> Result<String, CallError> {
        let values = self
            .0
            .call(Self::INTERFACE, "greet-slow", vec![Val::from(name)])
            .await?;
        match values.as_slice() {
            [Val::String(value)] => Ok(value.clone()),
            _ => Err(CallError::trap("reload greeter returned the wrong shape")),
        }
    }
}
