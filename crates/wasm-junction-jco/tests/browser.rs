//! Browser-only engine and JavaScript Promise Integration tests.

#![cfg(target_family = "wasm")]

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use wasm_encoder::{CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection};
use wasm_encoder::{ImportSection, Instruction, Module, TypeSection, ValType};

wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[wasm_bindgen(inline_js = r#"
export async function jspiSmoke(bytes) {
  if (typeof WebAssembly.Suspending !== 'function' ||
      typeof WebAssembly.promising !== 'function') {
    throw new Error('WebAssembly JSPI is unavailable');
  }
  const module = await WebAssembly.compile(bytes);
  const delayed = () => new Promise(resolve => setTimeout(() => resolve(41), 10));
  const instance = new WebAssembly.Instance(module, {
    host: { delayed: new WebAssembly.Suspending(delayed) },
  });
  return WebAssembly.promising(instance.exports.run)();
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = jspiSmoke)]
    async fn jspi_smoke(bytes: Uint8Array) -> Result<JsValue, JsValue>;
}

#[wasm_bindgen_test]
async fn suspends_and_resumes_a_core_wasm_call() {
    let result = jspi_smoke(Uint8Array::from(jspi_module().as_slice()))
        .await
        .unwrap();
    assert_eq!(result.as_f64(), Some(42.0));
}

fn jspi_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I32]);
    module.section(&types);
    let mut imports = ImportSection::new();
    imports.import("host", "delayed", EntityType::Function(0));
    module.section(&imports);
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    module.section(&exports);
    let mut code = CodeSection::new();
    let mut run = Function::new([]);
    run.instruction(&Instruction::Call(0));
    run.instruction(&Instruction::I32Const(1));
    run.instruction(&Instruction::I32Add);
    run.instruction(&Instruction::End);
    code.function(&run);
    module.section(&code);
    module.finish()
}
