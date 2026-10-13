use std::future::Future;

use tokio::sync::{mpsc, oneshot};
use wasm_junction::{Call, CallError, Middleware, Next, Vals};

use super::{Event, Policy, Verdict, Why};
use crate::console::{ConsentRequest, Console};

pub(crate) struct PolicyMiddleware {
    policy: Policy,
    console: Console,
    consent: Option<mpsc::UnboundedSender<ConsentRequest>>,
    model_host: String,
    verbose: bool,
}

impl PolicyMiddleware {
    pub(crate) fn new(policy: Policy, console: Console, model_host: String, verbose: bool) -> Self {
        Self {
            policy,
            console,
            consent: None,
            model_host,
            verbose,
        }
    }

    pub(crate) fn with_consent(mut self, consent: mpsc::UnboundedSender<ConsentRequest>) -> Self {
        self.consent = Some(consent);
        self
    }

    fn trace(&self, line: &str) -> Result<(), CallError> {
        self.console
            .line(line)
            .map_err(|error| CallError::trap(error.to_string()))
    }

    fn refuse(&self, why: &Why, status: &str) -> Result<Vals, CallError> {
        let reason = why.reasons.join("; ");
        self.trace(&format!("  ✗ {} {status} — {reason}", why.effect))?;
        Err(CallError::refused(reason))
    }

    fn routine(&self, event: &Event) -> bool {
        event.caller == "agent" && event.arguments.get(3) == Some(&self.model_host)
            || event.caller == "notes-tools"
                && event
                    .arguments
                    .get(2)
                    .is_some_and(|path| self.policy.base.join("notes").join(path).is_dir())
    }

    pub(super) async fn enforce<F, T>(&self, call: Call, next: F) -> Result<Vals, CallError>
    where
        F: FnOnce(Call) -> T,
        T: Future<Output = Result<Vals, CallError>>,
    {
        let event = match Event::from_call(&call, &self.policy.base) {
            Ok(event) => event,
            Err(reason) => {
                return self.refuse(
                    &Why {
                        effect: "resolve path".into(),
                        reasons: vec![reason],
                    },
                    "denied",
                );
            }
        };
        match self.policy.decide(&event).map_err(CallError::trap)? {
            Verdict::Allow(effect) => {
                if !effect.is_empty() && (self.verbose || !self.routine(&event)) {
                    self.trace(&format!("  · {effect}   allowed"))?;
                }
                next(call).await
            }
            Verdict::Deny(why) => self.refuse(&why, "denied"),
            Verdict::Ask(why) => {
                let Some(consent) = &self.consent else {
                    return self.refuse(&why, "denied");
                };
                let (answer, reply) = oneshot::channel();
                consent
                    .send(ConsentRequest {
                        effect: why.effect.clone(),
                        reason: why.reasons.join("; "),
                        answer,
                    })
                    .map_err(|_| CallError::trap("consent prompt is unavailable"))?;
                let allowed = reply
                    .await
                    .map_err(|_| CallError::trap("consent prompt closed"))?;
                if !allowed {
                    return Err(CallError::refused(why.reasons.join("; ")));
                }
                self.policy.approve(&event).map_err(CallError::trap)?;
                next(call).await
            }
        }
    }
}

impl Middleware for PolicyMiddleware {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        self.enforce(call, |call| next.run(call)).await
    }
}
