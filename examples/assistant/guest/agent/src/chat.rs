use serde_json::{Value, json};

const SYSTEM: &str = "You are a research assistant. Use list_notes to discover notes, read_note to read one by path, write_note to create or replace one, and fetch to retrieve a URL.";

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

pub enum Reply {
    Final(String),
    Tools {
        message: Value,
        calls: Vec<ToolCall>,
    },
}

pub fn system_message() -> Value {
    json!({"role": "system", "content": SYSTEM})
}

pub fn request(model: &str, messages: &[Value]) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "model": model,
        "messages": messages,
        "tools": [
            tool("list_notes", "List the available note paths", json!({
                "type": "object", "properties": {}, "additionalProperties": false
            })),
            tool("read_note", "Read a note by path", json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
                "additionalProperties": false
            })),
            tool("write_note", "Create or replace a note", json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "text": {"type": "string"}
                },
                "required": ["path", "text"],
                "additionalProperties": false
            })),
            tool("fetch", "Fetch the text at an HTTP or HTTPS URL", json!({
                "type": "object",
                "properties": {"url": {"type": "string"}},
                "required": ["url"],
                "additionalProperties": false
            }))
        ],
        "stream": false,
        "max_tokens": 1024
    }))
    .map_err(|error| error.to_string())
}

pub fn reply(body: &[u8]) -> Result<Reply, String> {
    let root: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let message = root
        .pointer("/choices/0/message")
        .ok_or_else(|| "model response has no message".to_owned())?;
    let calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if calls.is_empty() {
        return message
            .get("content")
            .and_then(Value::as_str)
            .map(|content| Reply::Final(content.to_owned()))
            .ok_or_else(|| "model response has no content".to_owned());
    }
    let calls = calls.iter().map(tool_call).collect::<Result<Vec<_>, _>>()?;
    Ok(Reply::Tools {
        message: json!({
            "role": "assistant",
            "content": message.get("content").cloned().unwrap_or(Value::Null),
            "tool_calls": message["tool_calls"]
        }),
        calls,
    })
}

fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {"name": name, "description": description, "parameters": parameters}
    })
}

fn tool_call(value: &Value) -> Result<ToolCall, String> {
    let field = |path: &str| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("tool call has no {path}"))
    };
    Ok(ToolCall {
        id: field("/id")?,
        name: field("/function/name")?,
        arguments: field("/function/arguments")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{Reply, reply, request, system_message};
    use serde_json::Value;

    #[test]
    fn handles_request_and_fixed_response_bodies() {
        let request: Value =
            serde_json::from_slice(&request("local-model", &[system_message()]).unwrap()).unwrap();
        assert_eq!(request["messages"][0]["role"], "system");
        assert_eq!(request["stream"], false);
        assert_eq!(request["max_tokens"], 1024);

        let tool = br#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_note","arguments":"{\"path\":\"research.md\"}"}}]}}]}"#;
        let Reply::Tools { calls, .. } = reply(tool).unwrap() else {
            panic!("expected tool calls");
        };
        assert_eq!(calls[0].name, "read_note");
        assert_eq!(calls[0].arguments, r#"{"path":"research.md"}"#);

        let final_reply =
            br#"{"choices":[{"message":{"role":"assistant","content":"A summary."}}]}"#;
        assert!(matches!(reply(final_reply).unwrap(), Reply::Final(text) if text == "A summary."));

        let extra = br#"{"choices":[{"message":{"role":"assistant","reasoning_content":"private chain","content":"Visible."}}]}"#;
        assert!(matches!(reply(extra).unwrap(), Reply::Final(text) if text == "Visible."));
    }
}
