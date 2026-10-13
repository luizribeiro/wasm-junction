use std::fs;
use wasm_junction::{Call, Caller, Val};

use super::event::value_text;
use super::{Event, Policy, Verdict, Why};

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
    let policy = Policy::load(&directory.join("policy.rego"), &[]).unwrap();
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
            reasons: vec!["private/output.md is outside notes/".into()],
        })
    );
}

#[test]
fn matches_filesystem_flags_exactly() {
    let policy = load_policy(&[]);
    assert_eq!(
        policy
            .decide(&open_at("notes/draft.md", &["requested-write-sync"]))
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
fn confines_files_and_hosts_to_their_declared_tools() {
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
}

#[test]
fn private_send_exceptions_use_history_trust_and_approval() {
    let private = open_at("private/diary.md", &["read"]);
    let outside = http_send("agent", "outside.example");
    let reason = "send to outside.example after reading private/diary.md";

    let policy = load_policy(&[]);
    assert!(matches!(policy.decide(&private), Ok(Verdict::Allow(_))));
    assert_eq!(
        policy.decide(&outside).unwrap(),
        Verdict::Ask(Why {
            effect: "send outside.example".into(),
            reasons: vec![reason.into()],
        })
    );
    policy.approve(&outside).unwrap();
    assert_eq!(
        policy.decide(&outside).unwrap(),
        Verdict::Allow("send outside.example".into())
    );

    let trusted = load_policy(&["model.example"]);
    assert!(matches!(trusted.decide(&private), Ok(Verdict::Allow(_))));
    assert_eq!(
        trusted
            .decide(&http_send("agent", "model.example"))
            .unwrap(),
        Verdict::Allow("send model.example".into())
    );

    assert_eq!(
        load_policy(&[]).decide(&outside).unwrap(),
        Verdict::Allow("send outside.example".into())
    );
}

#[test]
fn writes_ask_only_after_web_fetches() {
    let write = open_at("notes/summary.md", &["write"]);

    let fetched = load_policy(&[]);
    assert!(matches!(
        fetched.decide(&http_send("web-tools", "news.example")),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        fetched.decide(&write).unwrap(),
        Verdict::Ask(Why {
            effect: "write notes/summary.md".into(),
            reasons: vec!["write notes/summary.md after fetching from news.example".into()],
        })
    );

    let model = load_policy(&[]);
    assert!(matches!(
        model.decide(&http_send("agent", "model.example")),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        model.decide(&write).unwrap(),
        Verdict::Allow("write notes/summary.md".into())
    );
}

fn load_policy(trusted_hosts: &[&str]) -> Policy {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    Policy::load(
        &directory.join("policy.rego"),
        &trusted_hosts
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
