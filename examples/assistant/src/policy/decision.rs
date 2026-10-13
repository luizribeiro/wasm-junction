use regorus::Value;

use super::{Decision, Event, State};

pub(super) fn evaluate(state: &mut State, event: &Event) -> Result<Decision, String> {
    let input = serde_json::json!({
        "event": event.json(),
        "history": state.history.iter().map(Event::json).collect::<Vec<_>>(),
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
