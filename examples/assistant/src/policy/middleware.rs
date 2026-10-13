use tokio::sync::oneshot;
use wasm_junction::{Call, CallError, Middleware, Next, Vals};

use super::{Policy, Verdict, Why};
use crate::console::ConsentRequest;

impl Policy {
    fn trace(&self, line: String) -> Result<(), CallError> {
        self.inner
            .lock()
            .map_err(|error| CallError::trap(error.to_string()))?
            .trace
            .push(line);
        Ok(())
    }

    fn refuse(&self, why: &Why, status: &str) -> Result<Vals, CallError> {
        let reason = why.reasons.join("; ");
        self.trace(format!("{}: {status} — {reason}", why.effect))?;
        Err(CallError::refused(reason))
    }
}

impl Middleware for Policy {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let Some(policy) = call.extensions().get::<Self>().cloned() else {
            return next.run(call).await;
        };
        let event = match super::Event::from_call(&call, &policy.base) {
            Ok(event) => event,
            Err(reason) => {
                return policy.refuse(
                    &Why {
                        effect: "resolve path".into(),
                        reasons: vec![reason],
                    },
                    "denied",
                );
            }
        };
        match policy.decide(&event).map_err(CallError::trap)? {
            Verdict::Allow(effect) => {
                if !effect.is_empty() {
                    policy.trace(format!("{effect}: allowed"))?;
                }
                next.run(call).await
            }
            Verdict::Deny(why) => policy.refuse(&why, "denied"),
            Verdict::Ask(why) => {
                let Some(consent) = &policy.consent else {
                    return policy.refuse(&why, "denied");
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
                let answer = if allowed { "yes" } else { "no" };
                policy.trace(format!(
                    "{}: asked → {answer} — {}",
                    why.effect,
                    why.reasons.join("; ")
                ))?;
                if !allowed {
                    return Err(CallError::refused(why.reasons.join("; ")));
                }
                policy.approve(&event).map_err(CallError::trap)?;
                next.run(call).await
            }
        }
    }
}
