use std::fs;

use wasm_junction::{Call, Caller, Val};

use super::event::value_text;
use super::{Event, Policy, Verdict};

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
