use std::fs;
use std::io::Cursor;
use tokio::sync::mpsc;
use wasm_junction::{Call, CallErrorKind, Caller, Val};

use super::event::value_text;
use super::{Event, Policy, PolicyMiddleware, Verdict, Why};
use crate::console::Console;

#[test]
fn builds_generic_events_with_fixed_structured_text() {
    let call = Call::new(
        Caller::Component("agent".into()),
        "notes-tools",
        "example:assistant/notes-tools@0.1.0",
        "inspect",
        vec![
            Val::Flags(vec!["read".into(), "write".into()]),
            Val::Record(vec![("name".into(), Val::String("note".into()))]),
        ],
    );
    let event = Event::from_call(&call, std::path::Path::new(".")).unwrap();
    assert_eq!(event.caller, "agent");
    assert_eq!(event.callee, "notes-tools");
    assert_eq!(event.arguments, ["{read, write}", "{name: note}"]);
    assert_eq!(
        value_text(&Val::Option(Some(Box::new("host.test".into())))),
        "host.test"
    );

    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let policy = Policy::load(&directory.join("policy.rego"), "model.example", &[]).unwrap();
    assert!(matches!(policy.decide(&event), Ok(Verdict::Allow(_))));
    policy.approve(&event).unwrap();
}

#[test]
fn builds_filesystem_events_with_resolved_paths() {
    let base = std::env::temp_dir().join(format!("wasm-junction-event-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("notes/private")).unwrap();
    fs::write(base.join("notes/private/diary.md"), "private").unwrap();
    let call = Call::new(
        Caller::Component("notes-tools".into()),
        "wasi:filesystem/types@0.3.0",
        "wasi:filesystem/types@0.3.0",
        "[method]descriptor.open-at",
        vec![
            Val::String("descriptor".into()),
            Val::Flags(Vec::new()),
            Val::String("./notes/../private/diary.md".into()),
            Val::Flags(Vec::new()),
            Val::Flags(vec!["read".into()]),
            Val::String("/notes".into()),
        ],
    );
    let event = Event::from_call(&call, &base).unwrap();
    assert_eq!(event.arguments[2], "private/diary.md");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn real_call_shapes_reach_the_expected_rules() {
    let policy = load_policy(&[]);
    let secret = open_at("private/passwords.md", &["read"]);
    assert_eq!(
        policy.decide(&secret).unwrap(),
        Verdict::Deny(Why {
            effect: "read private/passwords.md".into(),
            reasons: vec!["private/passwords.md is secret".into()],
        })
    );

    let outside = open_at("private/output.md", &["write"]);
    assert_eq!(
        policy.decide(&outside).unwrap(),
        Verdict::Deny(Why {
            effect: "write private/output.md".into(),
            reasons: vec!["private/output.md is outside drafts/".into()],
        })
    );
}

#[test]
fn matches_filesystem_flags_exactly() {
    let policy = load_policy(&[]);
    assert_eq!(
        policy
            .decide(&open_at("drafts/draft.md", &["requested-write-sync"]))
            .unwrap(),
        Verdict::Allow(String::new())
    );
}

#[test]
fn weather_tools_cannot_write_notes() {
    let call = filesystem_call("weather-tools", "notes/weather.md", &["write"]);
    let event = Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert!(matches!(
        load_policy(&[]).decide(&event).unwrap(),
        Verdict::Deny(Why { reasons, .. })
            if reasons.contains(&"weather-tools may not write notes".to_owned())
    ));
}

#[test]
fn allowed_hosts_deny_by_default() {
    let policy = load_policy(&[]);
    assert_eq!(
        policy
            .decide(&open_at_from(
                "weather-tools",
                "private/diary.md",
                &["read"]
            ))
            .unwrap(),
        Verdict::Deny(Why {
            effect: "read private/diary.md".into(),
            reasons: vec!["weather-tools may not read notes".into()],
        })
    );
    assert!(matches!(
        policy.decide(&http_send("weather-tools", "api.open-meteo.com")),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        policy
            .decide(&http_send("weather-tools", "collector.example"))
            .unwrap(),
        Verdict::Deny(Why {
            effect: "send collector.example".into(),
            reasons: vec!["weather-tools may not send to collector.example".into()],
        })
    );
    assert!(matches!(
        policy.decide(&http_send("web-tools", "anywhere.test")),
        Ok(Verdict::Allow(_))
    ));
    for caller in ["notes-tools", "unknown-tools"] {
        assert!(matches!(
            policy.decide(&http_send(caller, "anywhere.test")),
            Ok(Verdict::Deny(_))
        ));
    }
    assert!(matches!(
        policy.decide(&http_send("agent", "outside.example")),
        Ok(Verdict::Deny(_))
    ));
}

#[test]
fn private_data_hosts_cover_any_allowed_request() {
    let private = open_at("private/diary.md", &["read"]);
    let model = http_send("agent", "model.example");
    let reason = "send to model.example with private/diary.md in the conversation";

    let policy = load_policy(&[]);
    assert!(matches!(policy.decide(&private), Ok(Verdict::Allow(_))));
    assert_eq!(
        policy.decide(&model).unwrap(),
        Verdict::Ask(Why {
            effect: "send model.example".into(),
            reasons: vec![reason.into()],
        })
    );

    let cleared = load_policy(&["model.example"]);
    assert!(matches!(cleared.decide(&private), Ok(Verdict::Allow(_))));
    assert_eq!(
        cleared
            .decide(&http_send("agent", "model.example"))
            .unwrap(),
        Verdict::Allow("send model.example".into())
    );

    assert!(matches!(
        cleared.decide(&http_send("web-tools", "model.example")),
        Ok(Verdict::Allow(_))
    ));
    assert!(matches!(
        cleared.decide(&http_send("web-tools", "example.com")),
        Ok(Verdict::Ask(_))
    ));
}

#[test]
fn writes_ask_only_after_web_fetches() {
    let write = open_at("drafts/summary.md", &["write"]);

    let fetched = load_policy(&[]);
    assert!(matches!(
        fetched.decide(&http_send("web-tools", "news.example")),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        fetched.decide(&write).unwrap(),
        Verdict::Ask(Why {
            effect: "write drafts/summary.md".into(),
            reasons: vec![
                "write drafts/summary.md with content from news.example in the conversation".into(),
            ],
        })
    );

    let model = load_policy(&[]);
    assert!(matches!(
        model.decide(&http_send("agent", "model.example")),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        model.decide(&write).unwrap(),
        Verdict::Allow("write drafts/summary.md".into())
    );
}

#[test]
fn dropped_turn_drops_its_private_label() {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    assert!(matches!(
        policy.decide(&open_at("private/diary.md", &["read"])),
        Ok(Verdict::Allow(_))
    ));
    assert!(matches!(
        policy.decide(&http_send("agent", "model.example")),
        Ok(Verdict::Ask(_))
    ));
    policy.end_turn(false).unwrap();

    policy.begin_turn().unwrap();
    assert_eq!(
        policy
            .decide(&http_send("web-tools", "example.com"))
            .unwrap(),
        Verdict::Allow("send example.com".into())
    );
}

#[test]
fn kept_turn_carries_its_private_label() {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    policy
        .decide(&open_at("private/diary.md", &["read"]))
        .unwrap();
    policy
        .approve(&http_send("agent", "model.example"))
        .unwrap();
    policy.end_turn(true).unwrap();

    policy.begin_turn().unwrap();
    assert!(matches!(
        policy.decide(&http_send("web-tools", "example.com")),
        Ok(Verdict::Ask(_))
    ));
}

fn load_policy(private_data_hosts: &[&str]) -> Policy {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    Policy::load(
        &directory.join("policy.rego"),
        "model.example",
        &private_data_hosts
            .iter()
            .map(|host| (*host).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn open_at(path: &str, flags: &[&str]) -> Event {
    open_at_from("notes-tools", path, flags)
}

fn open_at_from(caller: &str, path: &str, flags: &[&str]) -> Event {
    let call = filesystem_call(caller, path, flags);
    Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn middleware_owns_policy_and_handles_consent() {
    assert!(consent_result(Some(true)).await.is_ok());
    for answer in [Some(false), None] {
        assert_eq!(
            consent_result(answer).await.unwrap_err().kind(),
            CallErrorKind::Refused
        );
    }
}

async fn consent_result(answer: Option<bool>) -> Result<Vec<Val>, wasm_junction::CallError> {
    let (sender, mut requests) = mpsc::unbounded_channel();
    let console = Console::new(Cursor::new(Vec::<u8>::new()), std::io::sink());
    let middleware =
        PolicyMiddleware::new(load_policy(&[]), console, "model.example".into(), false);
    let middleware = if answer.is_some() {
        middleware.with_consent(sender)
    } else {
        middleware
    };
    middleware
        .enforce(
            filesystem_call("notes-tools", "private/diary.md", &["read"]),
            stub_next,
        )
        .await?;
    let send = middleware.enforce(http_call("agent", "model.example"), stub_next);
    if let Some(answer) = answer {
        let reply = async move {
            requests.recv().await.unwrap().answer.send(answer).unwrap();
        };
        tokio::join!(send, reply).0
    } else {
        send.await
    }
}

async fn stub_next(call: Call) -> Result<Vec<Val>, wasm_junction::CallError> {
    Ok(call.args)
}

fn filesystem_call(caller: &str, path: &str, flags: &[&str]) -> Call {
    Call::new(
        Caller::Component(caller.into()),
        "wasi:filesystem/types@0.3.0",
        "wasi:filesystem/types@0.3.0",
        "[method]descriptor.open-at",
        vec![
            Val::String("descriptor".into()),
            Val::Flags(Vec::new()),
            Val::String(path.into()),
            Val::Flags(Vec::new()),
            Val::Flags(flags.iter().map(|flag| (*flag).to_owned()).collect()),
            Val::String("/notes".into()),
        ],
    )
}

fn http_call(caller: &str, host: &str) -> Call {
    Call::new(
        Caller::Component(caller.into()),
        "wasi:http/client@0.3.0",
        "wasi:http/client@0.3.0",
        "send",
        vec![
            Val::String("request".into()),
            Val::Variant {
                case: "post".into(),
                value: None,
            },
            Val::Option(Some(Box::new(Val::Variant {
                case: "HTTP".into(),
                value: None,
            }))),
            Val::Option(Some(Box::new(Val::String(host.into())))),
        ],
    )
}

fn http_send(caller: &str, host: &str) -> Event {
    let call = http_call(caller, host);
    Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}
