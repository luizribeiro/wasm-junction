//! Browser component engine backed by jco-generated JavaScript.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use js_component_bindgen::{AsyncMode, InstantiationMode, TranspileOpts, transpile};

#[cfg(any(test, target_family = "wasm"))]
mod types;
#[cfg(target_family = "wasm")]
mod values;

#[cfg(all(test, target_family = "wasm"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[derive(Debug)]
struct TranspiledComponent {
    source: String,
    modules: Vec<(String, Vec<u8>)>,
    #[cfg(any(test, target_family = "wasm"))]
    signatures: types::Signatures,
}

fn transpile_component(bytes: &[u8]) -> Result<TranspiledComponent, String> {
    #[cfg(any(test, target_family = "wasm"))]
    let signatures = types::Signatures::from_component(bytes)
        .map_err(|error| format!("could not transpile WebAssembly component: {error}"))?;
    let output = transpile(
        bytes,
        TranspileOpts {
            name: "component".to_owned(),
            no_typescript: true,
            instantiation_mode: Some(InstantiationMode::Async),
            nodejs_compat_disabled: true,
            async_mode: Some(AsyncMode::JavaScriptPromiseIntegration {
                imports: vec!["*".to_owned()],
                exports: vec!["*".to_owned()],
            }),
            ..TranspileOpts::default()
        },
    )
    .map_err(|error| format!("could not transpile WebAssembly component: {error:#}"))?;

    let mut source = None;
    let mut modules = Vec::new();
    for (name, bytes) in output.files {
        let extension = std::path::Path::new(&name).extension();
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("js")) {
            source = Some(
                String::from_utf8(bytes)
                    .map_err(|error| format!("jco generated invalid UTF-8 JavaScript: {error}"))?,
            );
        } else if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("wasm")) {
            modules.push((name, bytes));
        }
    }
    source
        .map(|source| TranspiledComponent {
            source,
            modules,
            #[cfg(any(test, target_family = "wasm"))]
            signatures,
        })
        .ok_or_else(|| "jco did not generate a JavaScript module".to_owned())
}

mod native;

pub use native::JcoEngine;

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    fn transpiles_every_conformance_component_for_browsers() {
        let components = [
            ("notes", wasm_junction_conformance::component()),
            (
                "translator",
                wasm_junction_conformance::translator_component(),
            ),
            ("writer", wasm_junction_conformance::writer_component()),
            ("cycle-a", wasm_junction_conformance::cycle_a_component()),
            ("cycle-b", wasm_junction_conformance::cycle_b_component()),
            ("resources", wasm_junction_conformance::resource_component()),
            ("streams", wasm_junction_conformance::stream_component()),
            (
                "reload-v1",
                wasm_junction_conformance::reload_v1_component(),
            ),
            (
                "reload-v2",
                wasm_junction_conformance::reload_v2_component(),
            ),
            (
                "reload-breaking",
                wasm_junction_conformance::reload_breaking_component(),
            ),
            (
                "reload-writer",
                wasm_junction_conformance::reload_writer_component(),
            ),
        ];
        for (_name, bytes) in components {
            let output = transpile_component(bytes).unwrap();
            assert!(!output.modules.is_empty());
            assert!(!output.source.lines().any(is_static_import));
            assert!(!output.source.contains("node:"));
        }
        let source = transpile_component(wasm_junction_conformance::component())
            .unwrap()
            .source;
        assert!(source.contains("WebAssembly.Suspending"));
        assert!(source.contains("WebAssembly.promising"));
        let output = transpile_component(wasm_junction_conformance::component()).unwrap();
        let signature = output
            .signatures
            .export(wasm_junction_conformance::SUMMARIZER, "echo")
            .unwrap();
        assert!(!signature.params.is_empty());
        assert!(signature.result.is_some());
        assert_eq!(types::ValueType::String.name(), "string");
    }

    #[test]
    fn refuses_non_component_bytes_with_context() {
        let error = transpile_component(b"not a component").unwrap_err();
        assert!(error.starts_with("could not transpile WebAssembly component:"));
    }

    fn is_static_import(line: &str) -> bool {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("import{")
    }
}
