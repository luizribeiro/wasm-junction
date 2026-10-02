//! Settings replacement at component call boundaries.

mod support;

use std::sync::{Arc, Mutex};

use support::{
    FakeEngine, SETTINGS_CALLER, SETTINGS_HOST, SETTINGS_TARGET, block_on, component_bytes,
};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, Component, InterfaceHandle, Provided, Provider,
    Val, Vals,
};

const SETTINGS_WIT: &str = r"
    package test:settings@1.0.0;
    interface host { read: func() -> string; }
    interface target { run: func() -> string; }
    interface caller { run: func() -> tuple<string, string>; }
    world target-component { import host; export target; }
    world caller-component { import host; import target; export caller; }
";

#[derive(Clone)]
struct Label(&'static str);

#[derive(Clone)]
struct Target(wasm_junction::Handle);

impl InterfaceHandle for Target {
    const INTERFACE: &'static str = SETTINGS_TARGET;

    fn from_app(app: App, component: Arc<str>) -> Self {
        Self(wasm_junction::Handle::new(app, component, Self::INTERFACE))
    }
}

impl Target {
    async fn run(&self) -> Result<Vals, CallError> {
        self.0.call(Self::INTERFACE, "run", Vec::new()).await
    }

    fn within(&self, context: &CallContext) -> Self {
        Self(self.0.within(context))
    }
}

#[derive(Clone, Default)]
struct SettingsHost {
    nested: Arc<Mutex<Option<Target>>>,
}

impl Provider for SettingsHost {
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            let label = cx.settings::<Label>().map_or("missing", |value| value.0);
            let nested = self.nested.lock().unwrap().clone();
            if cx.caller().to_string() == "within-caller" {
                return nested.unwrap().within(cx).run().await;
            }
            Ok(vec![Val::from(label)])
        })
    }
}

fn component(world: &str, name: &str) -> Component {
    Component::from_bytes(component_bytes(SETTINGS_WIT, world))
        .unwrap()
        .named(name)
}

fn app(host: SettingsHost) -> App {
    App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(SETTINGS_HOST, host))
        .build()
        .unwrap()
}

#[test]
fn routed_components_use_their_own_settings() {
    let host = SettingsHost::default();
    let app = app(host.clone());
    app.configure("caller", Label("A")).unwrap();
    app.configure("callee", Label("B")).unwrap();
    block_on(app.load_all([
        component("caller-component", "caller"),
        component("target-component", "callee"),
    ]))
    .unwrap();

    let result = block_on(app.call("caller", SETTINGS_CALLER, "run", Vec::new())).unwrap();
    assert_eq!(result, [Val::Tuple(vec![Val::from("A"), Val::from("B")])]);
}

#[test]
fn within_uses_the_reached_components_settings() {
    let host = SettingsHost::default();
    let app = app(host.clone());
    for (name, label) in [("within-caller", "A"), ("callee", "B")] {
        app.configure(name, Label(label)).unwrap();
        block_on(app.load(component("target-component", name))).unwrap();
    }
    *host.nested.lock().unwrap() = Some(app.get::<Target>("callee").unwrap());

    let result = block_on(app.call("within-caller", SETTINGS_TARGET, "run", Vec::new())).unwrap();
    assert_eq!(result, [Val::from("B")]);
}
