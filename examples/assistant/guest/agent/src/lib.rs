wit_bindgen::generate!({
    path: "../../wit",
    world: "agent-component",
});

mod chat;
mod http;

use chat::{Reply, ToolCall};
use example::assistant::{notes_tools, weather_tools, web_tools};
use exports::example::assistant::agent::{Message, Role};
use serde_json::{Value, json};

const MAX_ROUNDS: usize = 4;

struct Component;

impl exports::example::assistant::agent::Guest for Component {
    async fn answer(conversation: Vec<Message>) -> Result<String, String> {
        let settings = Settings::read()?;
        let mut messages = vec![chat::system_message()];
        messages.extend(conversation.into_iter().map(message));
        for _ in 0..MAX_ROUNDS {
            let request = chat::request(&settings.model, &messages)?;
            let response = http::post(
                &settings.scheme,
                &settings.authority,
                &settings.path_prefix,
                &settings.api_key,
                request,
            )
            .await?;
            match chat::reply(&response)? {
                Reply::Final(answer) => return Ok(answer),
                Reply::Tools { message, calls } => {
                    messages.push(message);
                    for call in calls {
                        let id = call.id.clone();
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": run_tool(call).await
                        }));
                    }
                }
            }
        }
        Err(format!("model exceeded the {MAX_ROUNDS}-round tool limit"))
    }
}

struct Settings {
    scheme: String,
    authority: String,
    path_prefix: String,
    api_key: String,
    model: String,
}

impl Settings {
    fn read() -> Result<Self, String> {
        let environment = wasip3::cli::environment::get_environment();
        let get = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| format!("missing component setting {name}"))
        };
        Ok(Self {
            scheme: get("OPENAI_SCHEME")?,
            authority: get("OPENAI_AUTHORITY")?,
            path_prefix: get("OPENAI_PATH_PREFIX")?,
            api_key: get("OPENAI_API_KEY")?,
            model: get("OPENAI_MODEL")?,
        })
    }
}

fn message(message: Message) -> Value {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    json!({"role": role, "content": message.content})
}

async fn run_tool(call: ToolCall) -> String {
    let result = match call.name.as_str() {
        "list_notes" => notes_tools::list_notes()
            .await
            .map(|notes| notes.join("\n")),
        "read_note" => match argument(&call.arguments, "path") {
            Ok(path) => notes_tools::read_note(path).await,
            Err(error) => Err(error),
        },
        "write_note" => match (
            argument(&call.arguments, "path"),
            argument(&call.arguments, "text"),
        ) {
            (Ok(path), Ok(text)) => notes_tools::write_note(path, text)
                .await
                .map(|()| "note written".to_owned()),
            (Err(error), _) | (_, Err(error)) => Err(error),
        },
        "fetch" => match argument(&call.arguments, "url") {
            Ok(url) => web_tools::fetch(url).await,
            Err(error) => Err(error),
        },
        "get_weather" => match argument(&call.arguments, "city") {
            Ok(city) => weather_tools::get_weather(city).await,
            Err(error) => Err(error),
        },
        name => Err(format!("unknown tool {name}")),
    };
    result.unwrap_or_else(|error| format!("tool error: {error}"))
}

fn argument(arguments: &str, name: &str) -> Result<String, String> {
    serde_json::from_str::<Value>(arguments)
        .map_err(|error| error.to_string())?
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing {name} argument"))
}

export!(Component);
