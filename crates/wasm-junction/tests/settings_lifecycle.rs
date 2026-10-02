//! Component settings lifecycle tests using an engine-neutral fake engine.

mod support;

use support::{FakeEngine, SETTINGS_HOST, SETTINGS_TARGET, block_on, component_bytes};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, Component, Provided, Provider, Val, Vals,
};

const SETTINGS_WIT: &str = r"
    package test:settings@1.0.0;
    interface host { read: func() -> string; }
    interface target { run: func() -> string; }
    world component { import host; export target; }
";

#[derive(Clone)]
struct Label(&'static str);

struct SettingsHost;

impl Provider for SettingsHost {
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            Ok(vec![Val::from(
                cx.settings::<Label>().map_or("missing", |value| value.0),
            )])
        })
    }
}

fn component(name: &str) -> Component {
    Component::from_bytes(component_bytes(SETTINGS_WIT, "component"))
        .unwrap()
        .named(name)
}

fn call(app: &App, name: &str) -> Vals {
    block_on(app.call(name, SETTINGS_TARGET, "run", Vec::new())).unwrap()
}

#[test]
fn reloads_keep_settings_and_unloads_clear_them() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(SETTINGS_HOST, SettingsHost))
        .build()
        .unwrap();
    for (name, label) in [("alpha", "A"), ("beta", "B")] {
        app.configure(name, Label(label)).unwrap();
        block_on(app.load(component(name))).unwrap();
    }

    block_on(app.reload("alpha", component("replacement"))).unwrap();
    block_on(app.reload_all([
        ("alpha", component("replacement-alpha")),
        ("beta", component("replacement-beta")),
    ]))
    .unwrap();
    assert_eq!(call(&app, "alpha"), [Val::from("A")]);
    assert_eq!(call(&app, "beta"), [Val::from("B")]);

    block_on(app.unload("alpha")).unwrap();
    block_on(app.unload_force("beta")).unwrap();
    for name in ["alpha", "beta"] {
        block_on(app.load(component(name))).unwrap();
        assert_eq!(call(&app, name), [Val::from("missing")]);
    }
}
