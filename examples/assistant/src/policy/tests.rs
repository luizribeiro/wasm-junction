use std::fs;
use std::io::Cursor;
use tokio::sync::mpsc;
use wasm_junction::{Call, CallErrorKind, Caller, Val};

use super::event::value_text;
use super::{Event, Policy, PolicyMiddleware, Verdict, Why};
use crate::console::Console;
use crate::test_support::SharedOutput;

#[test]
fn builds_generic_events_with_fixed_structured_text() {
    let call = Call::new(
        Caller::Component("agent".into()),
        "notes-tools",
        "example:assistant/notes-tools@0.1.0",
        "list-notes",
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
            reasons: vec!["notes-tools may not write private/output.md".into()],
        })
    );
}

#[test]
fn every_open_is_a_read_and_mutating_flags_make_it_a_write() {
    let policy = load_policy(&[]);
    assert_eq!(
        policy
            .decide(&open_at_from("weather-tools", "research.md", &[]))
            .unwrap(),
        Verdict::Deny(Why {
            effect: "read research.md".into(),
            reasons: vec!["weather-tools may not read research.md".into()],
        })
    );
    assert_eq!(
        policy
            .decide(&open_at_with("draft.md", &["truncate"], &[]))
            .unwrap(),
        Verdict::Deny(Why {
            effect: "write draft.md".into(),
            reasons: vec!["notes-tools may not write draft.md".into()],
        })
    );
    assert_eq!(
        policy
            .decide(&open_at_with("draft.md", &["create"], &[]))
            .unwrap(),
        Verdict::Deny(Why {
            effect: "write draft.md".into(),
            reasons: vec!["notes-tools may not write draft.md".into()],
        })
    );
    for event in [
        open_at_with("private", &["directory", "create"], &["read"]),
        open_at_with("private", &["directory"], &["read", "write"]),
    ] {
        assert_eq!(
            policy.decide(&event).unwrap(),
            Verdict::Deny(Why {
                effect: "write private".into(),
                reasons: vec!["notes-tools may not write private".into()],
            })
        );
    }
}

#[test]
fn allows_only_declared_tool_calls() {
    let policy = load_policy(&[]);
    assert!(matches!(
        policy.decide(&tool_call("agent", "notes-tools", "read-note")),
        Ok(Verdict::Allow(_))
    ));
    for event in [
        tool_call("weather-tools", "notes-tools", "read-note"),
        tool_call("notes-tools", "agent", "answer"),
    ] {
        assert!(matches!(policy.decide(&event), Ok(Verdict::Deny(_))));
    }
}

#[test]
fn allows_only_declared_components_to_read_their_environment() {
    let policy = load_policy(&[]);
    assert_eq!(
        policy.environment("agent").unwrap(),
        &[
            "OPENAI_API_KEY".to_owned(),
            "OPENAI_AUTHORITY".to_owned(),
            "OPENAI_MODEL".to_owned(),
            "OPENAI_PATH_PREFIX".to_owned(),
            "OPENAI_SCHEME".to_owned(),
        ]
        .into_iter()
        .collect()
    );
    assert!(matches!(
        policy.decide(&environment_call("agent")),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        policy.decide(&environment_call("weather-tools")).unwrap(),
        Verdict::Deny(Why {
            effect: "read its environment".into(),
            reasons: vec!["weather-tools may not read its environment".into()],
        })
    );
}

#[test]
fn allows_only_notes_tools_to_list_the_notes_directory() {
    let policy = load_policy(&[]);
    assert_eq!(
        policy.decide(&read_directory("notes-tools")).unwrap(),
        Verdict::Allow("list /notes".into())
    );
    assert_eq!(
        policy.decide(&read_directory("weather-tools")).unwrap(),
        Verdict::Deny(Why {
            effect: "list /notes".into(),
            reasons: vec!["weather-tools may not list /notes".into()],
        })
    );
    let call = filesystem_call_with("weather-tools", "private", &["directory"], &["read"]);
    let event = Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert_eq!(
        policy.decide(&event).unwrap(),
        Verdict::Deny(Why {
            effect: "list private".into(),
            reasons: vec!["weather-tools may not list private".into()],
        })
    );
}

#[test]
fn allows_only_notes_tools_to_inspect_filesystem_metadata() {
    let policy = load_policy(&[]);
    for (function, arguments, path) in stat_call_shapes() {
        for (caller, expected) in [
            ("notes-tools", Verdict::Allow(format!("inspect {path}"))),
            (
                "weather-tools",
                Verdict::Deny(Why {
                    effect: format!("inspect {path}"),
                    reasons: vec![format!("weather-tools may not inspect {path}")],
                }),
            ),
        ] {
            let event = filesystem_event(caller, function, arguments.clone());
            assert_eq!(policy.decide(&event).unwrap(), expected);
        }
    }
}

#[test]
fn denies_unlisted_filesystem_and_http_calls_by_name() {
    let policy = load_policy(&[]);
    for (interface, function) in [
        ("wasi:filesystem/types@0.2.12", "[method]descriptor.open-at"),
        ("wasi:http/outgoing-handler@0.2.12", "handle"),
        (
            "wasi:filesystem/types@0.3.0",
            "[method]descriptor.rename-at",
        ),
        ("wasi:filesystem/types@0.3.0", "[method]descriptor.link-at"),
        ("example:unknown/interface@1.0.0", "surprise"),
    ] {
        let call = format!("{interface}.{function}");
        assert_eq!(
            policy.decide(&raw_event(interface, function)).unwrap(),
            Verdict::Deny(Why {
                effect: format!("call {call}"),
                reasons: vec![format!("{call} is not allowed by the policy")],
            })
        );
    }
}

#[test]
fn names_denied_filesystem_removals_and_creation() {
    let policy = load_policy(&[]);
    for (function, path, effect) in [
        (
            "[method]descriptor.unlink-file-at",
            "private/diary.md",
            "delete private/diary.md",
        ),
        (
            "[method]descriptor.remove-directory-at",
            "private",
            "delete private",
        ),
        (
            "[method]descriptor.create-directory-at",
            "new",
            "create directory new",
        ),
    ] {
        let event = filesystem_event(
            "notes-tools",
            function,
            vec!["descriptor".into(), path.into(), "/notes".into()],
        );
        assert_eq!(
            policy.decide(&event).unwrap(),
            Verdict::Deny(Why {
                effect: effect.into(),
                reasons: vec![format!("notes-tools may not {effect}")],
            })
        );
    }
}

#[test]
fn refuses_unresolved_removal_paths() {
    let call = filesystem_call_for(
        "notes-tools",
        "[method]descriptor.unlink-file-at",
        vec!["descriptor".into(), "../outside".into(), "/notes".into()],
    );
    assert!(Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn traces_a_denied_unknown_call_by_name() {
    let output = SharedOutput::default();
    let console = Console::new(Cursor::new(Vec::<u8>::new()), output.clone());
    let middleware =
        PolicyMiddleware::new(load_policy(&[]), console, "model.example".into(), false);
    let call = Call::new(
        Caller::Component("agent".into()),
        "unknown",
        "example:unknown/interface@1.0.0",
        "surprise",
        Vec::new(),
    );
    assert_eq!(
        middleware
            .enforce(call, stub_next)
            .await
            .unwrap_err()
            .kind(),
        CallErrorKind::Refused
    );
    assert_eq!(
        output.text(),
        "  ✗ call example:unknown/interface@1.0.0.surprise denied — example:unknown/interface@1.0.0.surprise is not allowed by the policy\n"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn traces_environment_reads_only_when_denied_or_verbose() {
    let output = SharedOutput::default();
    let console = Console::new(Cursor::new(Vec::<u8>::new()), output.clone());
    let middleware =
        PolicyMiddleware::new(load_policy(&[]), console, "model.example".into(), false);
    assert!(
        middleware
            .enforce(environment_call_for("agent"), stub_next)
            .await
            .is_ok()
    );
    assert_eq!(output.text(), "");
    assert_eq!(
        middleware
            .enforce(environment_call_for("weather-tools"), stub_next)
            .await
            .unwrap_err()
            .kind(),
        CallErrorKind::Refused
    );
    assert_eq!(
        output.text(),
        "  ✗ read its environment denied — weather-tools may not read its environment\n"
    );

    let verbose_output = SharedOutput::default();
    let verbose = PolicyMiddleware::new(
        load_policy(&[]),
        Console::new(Cursor::new(Vec::<u8>::new()), verbose_output.clone()),
        "model.example".into(),
        true,
    );
    assert!(
        verbose
            .enforce(environment_call_for("agent"), stub_next)
            .await
            .is_ok()
    );
    assert_eq!(
        verbose_output.text(),
        "  · read its environment   allowed\n"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn escapes_a_bidi_override_in_a_denied_path() {
    let output = SharedOutput::default();
    let console = Console::new(Cursor::new(Vec::<u8>::new()), output.clone());
    let middleware =
        PolicyMiddleware::new(load_policy(&[]), console, "model.example".into(), false);
    let call = filesystem_call("weather-tools", "private/\u{202e}txt.md", &["read"]);
    assert_eq!(
        middleware
            .enforce(call, stub_next)
            .await
            .unwrap_err()
            .kind(),
        CallErrorKind::Refused
    );
    assert_eq!(
        output.text(),
        "  ✗ read private/\\u{202e}txt.md denied — weather-tools may not read private/\\u{202e}txt.md\n"
    );
}

#[test]
fn weather_tools_cannot_write_notes() {
    let call = filesystem_call("weather-tools", "notes/weather.md", &["write"]);
    let event = Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert!(matches!(
        load_policy(&[]).decide(&event).unwrap(),
        Verdict::Deny(Why { reasons, .. })
            if reasons.contains(&"weather-tools may not write notes/weather.md".to_owned())
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
            reasons: vec!["weather-tools may not read private/diary.md".into()],
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
            effect: "make a request to https://collector.example/".into(),
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
fn does_not_ask_for_a_request_without_an_allow_rule() {
    let policy = private_conversation();
    policy.begin_turn().unwrap();
    assert_eq!(
        policy
            .decide(&http_send("notes-tools", "model.example"))
            .unwrap(),
        Verdict::Deny(Why {
            effect: "make a request to https://model.example/".into(),
            reasons: vec!["notes-tools may not send to model.example".into()],
        })
    );
}

#[test]
fn private_data_hosts_cover_any_allowed_request() {
    let private = open_at("private/diary.md", &["read"]);
    let model = http_send("agent", "model.example");
    let reason = "conversation includes private/diary.md";

    let policy = load_policy(&[]);
    assert!(matches!(policy.decide(&private), Ok(Verdict::Allow(_))));
    let verdict = policy.decide(&model).unwrap();
    assert_eq!(
        verdict,
        ask("agent", "make a request to model.example", &[reason])
    );
    let Verdict::Ask(_, consent) = verdict else {
        unreachable!()
    };
    assert!(matches!(
        policy.approve(&model, &consent),
        Ok(Verdict::Allow(_))
    ));
    assert_eq!(
        policy.decide(&model).unwrap(),
        Verdict::Allow("make a request to model.example".into())
    );
    assert!(matches!(
        policy.decide(&http_send("web-tools", "example.com")),
        Ok(Verdict::Ask(..))
    ));

    let cleared = load_policy(&["model.example"]);
    assert!(matches!(cleared.decide(&private), Ok(Verdict::Allow(_))));
    assert_eq!(
        cleared
            .decide(&http_send("agent", "model.example"))
            .unwrap(),
        Verdict::Allow("make a request to model.example".into())
    );
    assert!(matches!(
        cleared.decide(&http_send("web-tools", "model.example")),
        Ok(Verdict::Allow(_))
    ));
    assert!(matches!(
        cleared.decide(&http_send("web-tools", "example.com")),
        Ok(Verdict::Ask(..))
    ));
}

#[test]
fn write_approvals_are_not_remembered() {
    let write = open_at("drafts/summary.md", &["write"]);

    let fetched = load_policy(&[]);
    fetched.begin_turn().unwrap();
    assert!(matches!(
        fetched.decide(&http_send("web-tools", "news.example")),
        Ok(Verdict::Allow(_))
    ));
    fetched.end_turn(true).unwrap();
    fetched.begin_turn().unwrap();
    let verdict = fetched.decide(&write).unwrap();
    assert_eq!(
        verdict,
        ask(
            "notes-tools",
            "write drafts/summary.md",
            &["conversation includes content from news.example"],
        )
    );
    let Verdict::Ask(_, consent) = verdict else {
        unreachable!()
    };
    assert!(matches!(
        fetched.approve(&write, &consent),
        Ok(Verdict::Allow(_))
    ));
    fetched.end_turn(true).unwrap();
    fetched.begin_turn().unwrap();
    assert!(matches!(fetched.decide(&write), Ok(Verdict::Ask(..))));
}

#[test]
fn refusal_is_forgotten_after_the_turn() {
    let policy = private_conversation();
    policy.begin_turn().unwrap();
    let fetch = http_send("web-tools", "example.com");
    let Verdict::Ask(_, consent) = policy.decide(&fetch).unwrap() else {
        unreachable!()
    };
    policy.reject(&consent).unwrap();
    assert!(matches!(policy.decide(&fetch), Ok(Verdict::Deny(_))));

    policy.end_turn(true).unwrap();
    policy.begin_turn().unwrap();
    assert!(matches!(policy.decide(&fetch), Ok(Verdict::Ask(..))));
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
        Ok(Verdict::Ask(..))
    ));
    policy.end_turn(false).unwrap();

    policy.begin_turn().unwrap();
    assert_eq!(
        policy
            .decide(&http_send("web-tools", "example.com"))
            .unwrap(),
        Verdict::Allow("make a request to https://example.com/".into())
    );
}

#[test]
fn listing_notes_does_not_label_the_conversation() {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    assert!(matches!(
        policy.decide(&tool_call("agent", "notes-tools", "list-notes")),
        Ok(Verdict::Allow(_))
    ));
    assert!(matches!(
        policy.decide(&read_directory_at("notes-tools", "/notes")),
        Ok(Verdict::Allow(_))
    ));
    for directory in ["private", "drafts"] {
        assert_eq!(
            policy
                .decide(&open_at_with(directory, &["directory"], &["read"]))
                .unwrap(),
            Verdict::Allow(format!("list {directory}"))
        );
        assert_eq!(
            policy
                .decide(&read_directory_at("notes-tools", directory))
                .unwrap(),
            Verdict::Allow(format!("list {directory}"))
        );
    }
    policy.end_turn(true).unwrap();

    policy.begin_turn().unwrap();
    assert_eq!(
        policy.decide(&http_send("agent", "model.example")).unwrap(),
        Verdict::Allow("make a request to model.example".into())
    );
}

#[test]
fn kept_turn_carries_its_private_label() {
    let policy = private_conversation();

    policy.begin_turn().unwrap();
    let call = http_call_to("web-tools", "example.com", "/search?q=private");
    let fetch = Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    assert_eq!(
        policy.decide(&fetch).unwrap(),
        ask(
            "web-tools",
            "make a request to https://example.com/search?q=private",
            &["conversation includes private/diary.md"],
        )
    );
}

#[test]
fn weather_output_makes_a_later_write_ask() {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    policy
        .decide(&http_send("weather-tools", "api.open-meteo.com"))
        .unwrap();
    policy.end_turn(true).unwrap();

    policy.begin_turn().unwrap();
    assert_eq!(
        policy
            .decide(&open_at("drafts/weather.md", &["write"]))
            .unwrap(),
        ask(
            "notes-tools",
            "write drafts/weather.md",
            &["conversation includes content from weather-tools"],
        )
    );
}

#[test]
fn reading_a_draft_marks_both_kinds_of_sensitive_content() {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    policy
        .decide(&open_at("drafts/timing.md", &["read"]))
        .unwrap();
    policy.end_turn(true).unwrap();

    policy.begin_turn().unwrap();
    assert!(matches!(
        policy.decide(&http_send("agent", "model.example")),
        Ok(Verdict::Ask(..))
    ));
    assert!(matches!(
        policy.decide(&open_at("drafts/other.md", &["write"])),
        Ok(Verdict::Ask(..))
    ));
}

#[test]
fn writing_a_draft_adds_nothing_to_the_conversation() {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    policy
        .decide(&open_at("drafts/timing.md", &["write"]))
        .unwrap();
    policy.end_turn(true).unwrap();

    policy.begin_turn().unwrap();
    assert!(matches!(
        policy.decide(&http_send("agent", "model.example")),
        Ok(Verdict::Allow(..))
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

fn private_conversation() -> Policy {
    let policy = load_policy(&[]);
    policy.begin_turn().unwrap();
    policy
        .decide(&open_at("private/diary.md", &["read"]))
        .unwrap();
    let model = http_send("agent", "model.example");
    let Verdict::Ask(_, consent) = policy.decide(&model).unwrap() else {
        unreachable!()
    };
    assert!(matches!(
        policy.approve(&model, &consent),
        Ok(Verdict::Allow(_))
    ));
    policy.end_turn(true).unwrap();
    policy
}

fn open_at(path: &str, flags: &[&str]) -> Event {
    open_at_from("notes-tools", path, flags)
}

fn raw_event(interface: &str, function: &str) -> Event {
    Event {
        caller: "agent".into(),
        callee: interface.into(),
        interface: interface.into(),
        function: function.into(),
        arguments: Vec::new(),
    }
}

fn tool_call(source: &str, target: &str, function: &str) -> Event {
    Event {
        caller: source.into(),
        callee: target.into(),
        interface: format!("example:assistant/{target}@0.1.0"),
        function: function.into(),
        arguments: Vec::new(),
    }
}

fn environment_call(caller: &str) -> Event {
    let call = environment_call_for(caller);
    Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn environment_call_for(caller: &str) -> Call {
    Call::new(
        Caller::Component(caller.into()),
        "wasi:cli/environment@0.3.0",
        "wasi:cli/environment@0.3.0",
        "get-environment",
        Vec::new(),
    )
}

fn read_directory(caller: &str) -> Event {
    read_directory_at(caller, "/notes")
}

fn read_directory_at(caller: &str, path: &str) -> Event {
    Event {
        caller: caller.into(),
        callee: "wasi:filesystem/types@0.3.0".into(),
        interface: "wasi:filesystem/types@0.3.0".into(),
        function: "[method]descriptor.read-directory".into(),
        arguments: vec!["<resource>".into(), path.into()],
    }
}

fn ask(caller: &str, effect: &str, reasons: &[&str]) -> Verdict {
    Verdict::Ask(
        Why {
            effect: effect.into(),
            reasons: reasons.iter().map(|reason| (*reason).into()).collect(),
        },
        reasons
            .iter()
            .map(|reason| format!("{caller}|{effect}|{reason}"))
            .collect(),
    )
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

#[tokio::test(flavor = "current_thread")]
async fn consent_is_rechecked_before_the_call_runs() {
    let policy = load_policy(&[]);
    policy
        .decide(&open_at("private/diary.md", &["read"]))
        .unwrap();
    let (sender, mut requests) = mpsc::unbounded_channel();
    let console = Console::new(Cursor::new(Vec::<u8>::new()), std::io::sink());
    let middleware = PolicyMiddleware::new(policy.clone(), console, "model.example".into(), false)
        .with_consent(sender);
    let send = middleware.enforce(http_call("web-tools", "example.com"), stub_next);
    let change_policy = async {
        let request = requests.recv().await.unwrap();
        policy
            .decide(&open_at("private/second.md", &["read"]))
            .unwrap();
        request.answer.send(true).unwrap();
    };
    let (result, ()) = tokio::join!(send, change_policy);
    assert_eq!(result.unwrap_err().kind(), CallErrorKind::Refused);
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
    filesystem_call_with(caller, path, &[], flags)
}

fn filesystem_event(caller: &str, function: &str, arguments: Vec<Val>) -> Event {
    let call = filesystem_call_for(caller, function, arguments);
    Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn filesystem_call_for(caller: &str, function: &str, arguments: Vec<Val>) -> Call {
    Call::new(
        Caller::Component(caller.into()),
        "wasi:filesystem/types@0.3.0",
        "wasi:filesystem/types@0.3.0",
        function,
        arguments,
    )
}

fn stat_call_shapes() -> Vec<(&'static str, Vec<Val>, &'static str)> {
    vec![
        (
            "[method]descriptor.stat",
            vec!["descriptor".into(), "/notes".into()],
            "/notes",
        ),
        (
            "[method]descriptor.stat-at",
            vec![
                "descriptor".into(),
                Val::Flags(Vec::new()),
                "private/diary.md".into(),
                "/notes".into(),
            ],
            "private/diary.md",
        ),
        (
            "[method]descriptor.metadata-hash",
            vec!["descriptor".into(), "/notes".into()],
            "/notes",
        ),
        (
            "[method]descriptor.metadata-hash-at",
            vec![
                "descriptor".into(),
                Val::Flags(Vec::new()),
                "private/diary.md".into(),
                "/notes".into(),
            ],
            "private/diary.md",
        ),
        (
            "[method]descriptor.readlink-at",
            vec![
                "descriptor".into(),
                "private/diary.md".into(),
                "/notes".into(),
            ],
            "private/diary.md",
        ),
        (
            "[method]descriptor.is-same-object",
            vec![
                "descriptor".into(),
                "other".into(),
                "/notes".into(),
                "/notes".into(),
            ],
            "/notes",
        ),
    ]
}

fn open_at_with(path: &str, open_flags: &[&str], descriptor_flags: &[&str]) -> Event {
    let call = filesystem_call_with("notes-tools", path, open_flags, descriptor_flags);
    Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn filesystem_call_with(
    caller: &str,
    path: &str,
    open_flags: &[&str],
    descriptor_flags: &[&str],
) -> Call {
    Call::new(
        Caller::Component(caller.into()),
        "wasi:filesystem/types@0.3.0",
        "wasi:filesystem/types@0.3.0",
        "[method]descriptor.open-at",
        vec![
            Val::String("descriptor".into()),
            Val::Flags(Vec::new()),
            Val::String(path.into()),
            Val::Flags(open_flags.iter().map(|flag| (*flag).to_owned()).collect()),
            Val::Flags(
                descriptor_flags
                    .iter()
                    .map(|flag| (*flag).to_owned())
                    .collect(),
            ),
            Val::String("/notes".into()),
        ],
    )
}

fn http_call(caller: &str, host: &str) -> Call {
    http_call_to(caller, host, "/")
}

fn http_call_to(caller: &str, host: &str, path: &str) -> Call {
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
                case: "HTTPS".into(),
                value: None,
            }))),
            Val::Option(Some(Box::new(Val::String(host.into())))),
            Val::Option(Some(Box::new(Val::String(path.into())))),
            Val::List(Vec::new()),
        ],
    )
}

fn http_send(caller: &str, host: &str) -> Event {
    let call = http_call(caller, host);
    Event::from_call(&call, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}
