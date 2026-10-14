use std::collections::{BTreeMap, BTreeSet};
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
    Ask(Why, BTreeSet<String>),
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
    environment: BTreeMap<String, BTreeSet<String>>,
}

pub(super) struct State {
    engine: Engine,
    turn: Vec<Event>,
    conversation: Vec<Label>,
    approved: BTreeSet<String>,
    refused: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Label {
    private: BTreeSet<String>,
    untrusted: BTreeSet<String>,
}

pub(super) struct Decision {
    verdict: String,
    reasons: Vec<String>,
    effect: String,
    consent: BTreeSet<String>,
    remember: bool,
}

impl Policy {
    pub(crate) fn load(
        path: &Path,
        model_host: &str,
        private_data_hosts: &[String],
    ) -> Result<Self, String> {
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
                    &serde_json::json!({
                        "model_host": model_host,
                        "private_data_hosts": private_data_hosts,
                    })
                    .to_string(),
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        let environment = engine
            .eval_rule("data.assistant.environment".into())
            .map_err(|error| error.to_string())?;
        let environment = serde_json::from_str(
            &environment
                .to_json_str()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("policy environment is invalid: {error}"))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(State {
                engine,
                turn: Vec::new(),
                conversation: Vec::new(),
                approved: BTreeSet::new(),
                refused: BTreeSet::new(),
            })),
            base: path.parent().unwrap_or_else(|| Path::new(".")).to_owned(),
            environment,
        })
    }

    pub(crate) fn environment(&self, component: &str) -> Option<&BTreeSet<String>> {
        self.environment.get(component)
    }

    pub(crate) fn decide(&self, event: &Event) -> Result<Verdict, String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        let decision = evaluate(&mut state, event).unwrap_or_else(|reason| Decision {
            verdict: "deny".into(),
            reasons: vec![format!("policy evaluation failed: {reason}")],
            effect: "evaluate policy".into(),
            consent: BTreeSet::new(),
            remember: false,
        });
        Ok(apply(&mut state, event, decision))
    }

    pub(crate) fn approve(
        &self,
        event: &Event,
        consent: &BTreeSet<String>,
    ) -> Result<Verdict, String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        let added = consent
            .difference(&state.approved)
            .cloned()
            .collect::<BTreeSet<_>>();
        state.approved.extend(added.iter().cloned());
        let decision = match evaluate(&mut state, event) {
            Ok(decision) => decision,
            Err(error) => {
                state.approved.retain(|key| !added.contains(key));
                return Err(error);
            }
        };
        if decision.verdict != "allow" || !decision.remember {
            state.approved.retain(|key| !added.contains(key));
        }
        Ok(apply(&mut state, event, decision))
    }

    pub(crate) fn reject(&self, consent: &BTreeSet<String>) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        state.refused.extend(consent.iter().cloned());
        Ok(())
    }

    pub(crate) fn begin_turn(&self) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        state.turn.clear();
        state.refused.clear();
        Ok(())
    }

    pub(crate) fn end_turn(&self, keep: bool) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        if keep {
            let label = decision::turn_label(&mut state)?;
            state.conversation.push(label);
        }
        state.turn.clear();
        state.refused.clear();
        Ok(())
    }
}

fn apply(state: &mut State, event: &Event, decision: Decision) -> Verdict {
    let why = Why {
        effect: decision.effect,
        reasons: decision.reasons,
    };
    match decision.verdict.as_str() {
        "allow" => {
            state.turn.push(event.clone());
            Verdict::Allow(why.effect)
        }
        "ask" => Verdict::Ask(why, decision.consent),
        "deny" => Verdict::Deny(why),
        verdict => Verdict::Deny(Why {
            effect: "evaluate policy".into(),
            reasons: vec![format!("policy returned unknown verdict {verdict}")],
        }),
    }
}

impl Label {
    fn join(&mut self, other: &Self) {
        self.private.extend(other.private.iter().cloned());
        self.untrusted.extend(other.untrusted.iter().cloned());
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({"private": self.private, "untrusted": self.untrusted})
    }
}

#[cfg(test)]
mod tests;
