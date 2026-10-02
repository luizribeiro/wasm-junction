use std::future::Future;
use std::sync::Arc;

use crate::{
    BoxFuture, Call, CallError, ChannelDirection, HostBound, InvocationId, MaybeSend, Vals,
};

/// A lifecycle notification observed by middleware.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A component was loaded under an application name.
    Load {
        /// The application component name.
        component: Arc<str>,
        /// The versioned interfaces exported by the component.
        exports: Vec<Arc<str>>,
    },
    /// A loaded component was removed.
    Unload {
        /// The application component name.
        component: Arc<str>,
    },
    /// A loaded component's generation was replaced.
    Reload {
        /// The application component name.
        component: Arc<str>,
        /// The interfaces exported by the previous generation.
        old_exports: Vec<Arc<str>>,
        /// The interfaces exported by the new generation.
        new_exports: Vec<Arc<str>>,
    },
    /// An invocation of the named component is beginning.
    InvocationStart {
        /// The application-local invocation identifier.
        invocation: InvocationId,
        /// The application name of the invoked component.
        component: Arc<str>,
    },
    /// An invocation of the named component has ended.
    InvocationEnd {
        /// The application-local invocation identifier.
        invocation: InvocationId,
        /// The application name of the invoked component.
        component: Arc<str>,
    },
    /// A guest's owned host-resource handle is gone.
    ///
    /// This event fires before the provider's drop runs and remains observable whether that drop
    /// succeeds or fails.
    ResourceDrop {
        /// The invocation that owned the resource.
        invocation: InvocationId,
        /// The resource's defining versioned interface.
        interface: Arc<str>,
        /// The resource name within its defining interface.
        resource: Arc<str>,
        /// The provider-assigned resource id.
        id: u32,
    },
    /// A byte stream began crossing the engine boundary.
    ChannelOpen {
        /// The invocation whose call opened the channel.
        invocation: InvocationId,
        /// The opaque stream id.
        stream: u64,
        /// The direction bytes travel.
        direction: ChannelDirection,
    },
    /// A byte stream stopped crossing the engine boundary.
    ChannelClose {
        /// The invocation whose call opened the channel.
        invocation: InvocationId,
        /// The opaque stream id.
        stream: u64,
        /// The direction bytes traveled.
        direction: ChannelDirection,
    },
}

/// Logic that wraps every host-to-guest and guest-to-host call.
pub trait Middleware: HostBound {
    /// Handles a call, optionally forwarding it through `next`.
    fn call(
        &self,
        call: Call,
        next: Next,
    ) -> impl Future<Output = Result<Vals, CallError>> + MaybeSend;

    /// Observes an invocation lifecycle event.
    fn event(&self, _event: &Event) {}
}

pub(crate) trait ErasedMiddleware: HostBound {
    fn call(&self, call: Call, next: Next) -> BoxFuture<'_, Result<Vals, CallError>>;
    fn event(&self, event: &Event);
}

impl<T: Middleware> ErasedMiddleware for T {
    fn call(&self, call: Call, next: Next) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(Middleware::call(self, call, next))
    }

    fn event(&self, event: &Event) {
        Middleware::event(self, event);
    }
}

pub(crate) trait CallTarget: HostBound {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>>;
}

/// The remainder of a middleware chain.
///
/// Clone this value when a middleware may retry the wrapped call.
#[derive(Clone)]
pub struct Next {
    middleware: Arc<[Arc<dyn ErasedMiddleware>]>,
    index: usize,
    target: Arc<dyn CallTarget>,
}

impl Next {
    pub(crate) fn new(
        middleware: Arc<[Arc<dyn ErasedMiddleware>]>,
        target: Arc<dyn CallTarget>,
    ) -> Self {
        Self {
            middleware,
            index: 0,
            target,
        }
    }

    /// Continues the chain with `call`.
    #[must_use]
    pub fn run(self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        if let Some(middleware) = self.middleware.get(self.index).cloned() {
            let next = Self {
                index: self.index + 1,
                ..self
            };
            Box::pin(async move { middleware.call(call, next).await })
        } else {
            self.target.call(call)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::{CallErrorKind, Caller, TypeError, TypedCall, Val};

    const NOTES: &str = "example:journal/notes@0.1.0";

    #[derive(Clone)]
    struct Read(String);

    impl TypedCall for Read {
        type Output = ();
        const INTERFACE: &'static str = NOTES;
        const FUNCTION: &'static str = "read";

        fn from_vals(values: &[Val]) -> Result<Self, TypeError> {
            let [Val::String(name)] = values else {
                return Err(TypeError::new("expected one string"));
            };
            Ok(Self(name.clone()))
        }

        fn into_vals(self) -> Vals {
            vec![self.0.into()]
        }

        fn output((): ()) -> Vals {
            Vec::new()
        }

        fn decode_output(values: &[Val]) -> Result<(), TypeError> {
            values
                .is_empty()
                .then_some(())
                .ok_or_else(|| TypeError::new("expected no values"))
        }
    }

    struct Record(&'static str, Arc<Mutex<Vec<&'static str>>>);

    impl Middleware for Record {
        async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
            self.1.lock().unwrap().push(self.0);
            let result = next.run(call).await;
            self.1.lock().unwrap().push(self.0);
            result
        }
    }

    struct Target(Arc<Mutex<Vec<&'static str>>>);

    impl CallTarget for Target {
        fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
            self.0.lock().unwrap().push("target");
            Box::pin(async move { Ok(call.args) })
        }
    }

    fn call(name: &str) -> Call {
        Call::new(Caller::Host, "notebook", NOTES, "read", vec![name.into()])
    }

    fn run(middleware: Vec<Arc<dyn ErasedMiddleware>>, call: Call) -> Result<Vals, CallError> {
        let trace = Arc::new(Mutex::new(Vec::new()));
        block_on(Next::new(middleware.into(), Arc::new(Target(trace))).run(call))
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }

    #[test]
    fn first_registered_middleware_is_outermost() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let middleware: Vec<Arc<dyn ErasedMiddleware>> = vec![
            Arc::new(Record("outer", trace.clone())),
            Arc::new(Record("inner", trace.clone())),
        ];
        let target = Arc::new(Target(trace.clone()));

        block_on(Next::new(middleware.into(), target).run(call("daily"))).unwrap();

        assert_eq!(
            *trace.lock().unwrap(),
            ["outer", "inner", "target", "inner", "outer"]
        );
    }

    #[test]
    fn middleware_can_short_circuit_rewrite_and_retry() {
        struct Behavior;
        impl Middleware for Behavior {
            async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
                let Some(read) = call.view::<Read>()? else {
                    return Err(CallError::trap("unexpected call"));
                };
                if read.0 == "secret" {
                    return Err(CallError::refused("access denied"));
                }
                call.set_view(Read(String::from("weekly")))?;
                let first = next.clone().run(call.clone()).await?;
                let second = next.run(call).await?;
                assert_eq!(first, second);
                Ok(second)
            }
        }

        assert_eq!(
            run(vec![Arc::new(Behavior)], call("daily")).unwrap(),
            [Val::from("weekly")]
        );
        let refusal = run(vec![Arc::new(Behavior)], call("secret")).unwrap_err();
        assert_eq!(refusal.kind(), CallErrorKind::Refused);
        assert_eq!(refusal.to_string(), "access denied");
    }

    #[test]
    fn middleware_can_use_values_without_a_typed_view() {
        struct Raw;
        impl Middleware for Raw {
            async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
                call.args = vec![Val::from("raw")];
                next.run(call).await
            }
        }

        assert_eq!(
            run(vec![Arc::new(Raw)], call("daily")).unwrap(),
            [Val::from("raw")]
        );
    }
}
