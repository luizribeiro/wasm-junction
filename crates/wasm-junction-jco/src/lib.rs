//! Browser component engine backed by jco-generated JavaScript.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use js_component_bindgen::{AsyncMode, InstantiationMode, TranspileOpts, transpile};
use wasm_junction_core::EngineError;

#[cfg(any(test, target_family = "wasm"))]
mod types;
#[cfg(target_family = "wasm")]
mod values;

#[derive(Debug)]
struct TranspiledComponent {
    source: String,
    modules: Vec<(String, Vec<u8>)>,
    #[cfg(any(test, target_family = "wasm"))]
    signatures: types::Signatures,
}

fn transpile_component(bytes: &[u8]) -> Result<TranspiledComponent, EngineError> {
    #[cfg(any(test, target_family = "wasm"))]
    let signatures = types::Signatures::from_component(bytes).map_err(|error| {
        EngineError::new(format!(
            "could not transpile WebAssembly component: {error}"
        ))
    })?;
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
    .map_err(|error| {
        EngineError::new(format!(
            "could not transpile WebAssembly component: {error:#}"
        ))
    })?;

    let mut source = None;
    let mut modules = Vec::new();
    for (name, bytes) in output.files {
        let extension = std::path::Path::new(&name).extension();
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("js")) {
            let generated_source = String::from_utf8(bytes).map_err(|error| {
                EngineError::new(format!("jco generated invalid UTF-8 JavaScript: {error}"))
            })?;
            let generated_source = repair_char_lowering(&generated_source)?;
            source = Some(guard_failed_imports(&generated_source)?);
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
        .ok_or_else(|| EngineError::new("jco did not generate a JavaScript module"))
}

fn guard_failed_imports(source: &str) -> Result<String, EngineError> {
    const HEADER: &str = "export function instantiate(getCoreModule, imports, instantiateCore = WebAssembly.instantiate) {";
    const STATE: &str =
        "const wasmJunctionImportState = imports['wasm-junction:internal/import-state'];";
    const GUARD: &str = "if (wasmJunctionImportState.poisoned) {\n\
         throw new WebAssembly.RuntimeError('component import called after an earlier import failed');\n\
         }";
    const DECLARATIONS: [&str; 2] = [
        "async function _lowerImport(args) {",
        "function _lowerImportBackwardsCompat(args) {",
    ];
    let uses_imports = source.contains("imports[") || source.contains("imports.");
    if !uses_imports {
        return Ok(source.to_owned());
    }
    if !DECLARATIONS
        .iter()
        .any(|declaration| source.contains(declaration))
    {
        return Err(EngineError::new(
            "jco generated imports without a recognized lowering function",
        ));
    }
    if !source.contains(HEADER) {
        return Err(EngineError::new(
            "jco generated an unrecognized instantiation function",
        ));
    }
    let mut source = source.replacen(HEADER, &format!("{HEADER}\n  {STATE}"), 1);
    for declaration in DECLARATIONS {
        if source.contains(declaration) {
            source = source.replace(declaration, &format!("{declaration}\n    {GUARD}"));
        }
    }
    Ok(source)
}

fn repair_char_lowering(source: &str) -> Result<String, EngineError> {
    const INTRINSIC: &str = "_lowerFlatChar";
    const BROKEN_LOWERING: &str =
        "new DataView(ctx.memory.buffer).setUint32(ctx.storagePtr, i32ToChar(ctx.vals[0]), true);";
    if !source.contains(INTRINSIC) {
        return Ok(source.to_owned());
    }
    if !source.contains(BROKEN_LOWERING) {
        return Err(EngineError::new(
            "jco generated an unrecognized `_lowerFlatChar` implementation",
        ));
    }
    // js-component-bindgen 2.13's lowering path calls the lifting helper `i32ToChar` on a string.
    Ok(source.replace(
        BROKEN_LOWERING,
        "const value = ctx.vals[0];\n\
         if (typeof value !== 'string' || [...value].length !== 1) {\n\
           throw new TypeError('invalid WIT char');\n\
         }\n\
         const codePoint = value.codePointAt(0);\n\
         if (codePoint >= 0xD800 && codePoint <= 0xDFFF) {\n\
           throw new TypeError('invalid WIT char');\n\
         }\n\
         new DataView(ctx.memory.buffer).setUint32(ctx.storagePtr, codePoint, true);",
    ))
}

#[cfg(target_family = "wasm")]
mod browser;
#[cfg(not(target_family = "wasm"))]
mod native;

#[cfg(target_family = "wasm")]
pub use browser::JcoEngine;
#[cfg(not(target_family = "wasm"))]
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
        assert!(source.contains("wasmJunctionImportState.poisoned"));
        assert!(source.contains("component import called after an earlier import failed"));
        assert!(source.contains("invalid WIT char"));
        assert!(source.contains("codePoint >= 0xD800 && codePoint <= 0xDFFF"));
        assert!(!source.contains("i32ToChar(ctx.vals[0])"));
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
        assert!(
            error
                .to_string()
                .starts_with("could not transpile WebAssembly component:")
        );
    }

    #[test]
    fn refuses_an_unrecognized_char_lowerer() {
        let error = repair_char_lowering("function _lowerFlatChar() { return 0; }").unwrap_err();
        assert_eq!(
            error.to_string(),
            "jco generated an unrecognized `_lowerFlatChar` implementation"
        );
    }

    #[test]
    fn leaves_source_without_a_char_lowerer_untouched() {
        let source = "export const answer = 42;";
        assert_eq!(repair_char_lowering(source).unwrap(), source);
    }

    #[test]
    fn refuses_imports_with_an_unrecognized_lowerer() {
        let source = "export function instantiate(getCoreModule, imports, instantiateCore = WebAssembly.instantiate) {\n\
                      const read = imports['example:notes/notes'].read;\n\
                      function renamedLowering(args) { return read(args); }\n\
                      }";
        let error = guard_failed_imports(source).unwrap_err();
        assert_eq!(
            error.to_string(),
            "jco generated imports without a recognized lowering function"
        );
    }

    #[test]
    fn leaves_source_without_imports_untouched() {
        let source = "export function instantiate(getCoreModule, imports, instantiateCore = WebAssembly.instantiate) { return {}; }";
        assert_eq!(guard_failed_imports(source).unwrap(), source);
    }

    fn is_static_import(line: &str) -> bool {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("import{")
    }
}
