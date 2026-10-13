use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use regorus::{Engine, Value};
mod decision;
mod event;
mod middleware;

use decision::evaluate;
pub(crate) use middleware::PolicyMiddleware;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Event {
    caller: String,
    callee: String,
    interface: String,
    function: String,
    arguments: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    Allow(String),
    Ask(Why),
    Deny(Why),
}

#[derive(Debug, PartialEq)]
pub(crate) struct Why {
    effect: String,
    reasons: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct Policy {
    inner: Arc<Mutex<State>>,
    base: PathBuf,
}

pub(super) struct State {
    engine: Engine,
    history: Vec<Event>,
    approved: BTreeSet<String>,
}

pub(super) struct Decision {
    verdict: String,
    reasons: Vec<String>,
    effect: String,
}

impl Policy {
    pub(crate) fn load(path: &Path, private_data_hosts: &[String]) -> Result<Self, String> {
        let mut engine = Engine::new();
        for policy in [path.to_owned(), path.with_file_name("wasi.rego")] {
            let source = std::fs::read_to_string(&policy).map_err(|error| error.to_string())?;
            engine
                .add_policy(policy.display().to_string(), source)
                .map_err(|error| error.to_string())?;
        }
        engine
            .add_data(
                Value::from_json_str(
                    &serde_json::json!({"private_data_hosts": private_data_hosts}).to_string(),
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            inner: Arc::new(Mutex::new(State {
                engine,
                history: Vec::new(),
                approved: BTreeSet::new(),
            })),
            base: path.parent().unwrap_or_else(|| Path::new(".")).to_owned(),
        })
    }

    pub(crate) fn decide(&self, event: &Event) -> Result<Verdict, String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        let decision = evaluate(&mut state, event).unwrap_or_else(|reason| Decision {
            verdict: "deny".into(),
            reasons: vec![format!("policy evaluation failed: {reason}")],
            effect: "evaluate policy".into(),
        });
        let why = Why {
            effect: decision.effect,
            reasons: decision.reasons,
        };
        match decision.verdict.as_str() {
            "allow" => {
                state.history.push(event.clone());
                Ok(Verdict::Allow(why.effect))
            }
            "ask" => Ok(Verdict::Ask(why)),
            "deny" => Ok(Verdict::Deny(why)),
            verdict => Ok(Verdict::Deny(Why {
                effect: "evaluate policy".into(),
                reasons: vec![format!("policy returned unknown verdict {verdict}")],
            })),
        }
    }

    pub(crate) fn approve(&self, event: &Event) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        let decision = evaluate(&mut state, event)?;
        state.approved.extend(decision.reasons);
        state.history.push(event.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests;
