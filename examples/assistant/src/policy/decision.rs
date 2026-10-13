use regorus::Value;

use std::collections::BTreeSet;

use super::{Decision, Event, Label, State};

pub(super) fn evaluate(state: &mut State, event: &Event) -> Result<Decision, String> {
    let mut label = state
        .conversation
        .iter()
        .fold(Label::default(), |mut label, turn| {
            label.join(turn);
            label
        });
    label.join(&turn_label(state)?);
    let input = serde_json::json!({
        "event": event.json(),
        "history": state.turn.iter().map(Event::json).collect::<Vec<_>>(),
        "label": label.json(),
        "approved": state.approved,
    });
    state
        .engine
        .set_input(Value::from_json_str(&input.to_string()).map_err(|error| error.to_string())?);
    let value = state
        .engine
        .eval_rule("data.assistant.decision".into())
        .map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&value.to_json_str().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let text = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("policy decision has no {name}"))
    };
    let reasons = value
        .get("reasons")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "policy decision has no reasons".to_owned())?
        .iter()
        .map(|reason| {
            reason
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "policy reason is not text".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Decision {
        verdict: text("verdict")?,
        reasons,
        effect: text("effect")?,
    })
}

pub(super) fn turn_label(state: &mut State) -> Result<Label, String> {
    let input = serde_json::json!({
        "history": state.turn.iter().map(Event::json).collect::<Vec<_>>(),
    });
    state
        .engine
        .set_input(Value::from_json_str(&input.to_string()).map_err(|error| error.to_string())?);
    let value = state
        .engine
        .eval_rule("data.assistant.turn_label".into())
        .map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&value.to_json_str().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    Ok(Label {
        private: strings(&value, "private")?,
        untrusted: strings(&value, "untrusted")?,
    })
}

fn strings(value: &serde_json::Value, name: &str) -> Result<BTreeSet<String>, String> {
    value
        .get(name)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("policy label has no {name}"))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("policy label {name} is not text"))
        })
        .collect()
}
