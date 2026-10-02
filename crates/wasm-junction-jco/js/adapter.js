// @ts-check

/**
 * @typedef {(...args: unknown[]) => unknown} ComponentFunction
 * @typedef {Record<string, Record<string, ComponentFunction>>} ComponentInstance
 * @typedef {{ instantiate: (
 *   getModule: (name: string) => WebAssembly.Module,
 *   imports: WebAssembly.Imports,
 * ) => Promise<ComponentInstance> }} ComponentNamespace
 * @typedef {{
 *   namespace: ComponentNamespace,
 *   modules: Map<string, WebAssembly.Module>,
 * }} ComponentRuntime
 * @typedef {(
 *   interfaceName: string,
 *   functionName: string,
 *   args: unknown[],
 * ) => Promise<unknown>} Dispatch
 */

/**
 * @param {string} source
 * @param {string[]} names
 * @param {BufferSource[]} modules
 * @returns {Promise<ComponentRuntime>}
 */
export async function compileComponent(source, names, modules) {
  const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  try {
    const namespace = /** @type {ComponentNamespace} */ (await import(url));
    const compiled = await Promise.all(modules.map(module => WebAssembly.compile(module)));
    return { namespace, modules: new Map(names.map((name, index) => [name, compiled[index]])) };
  } finally {
    URL.revokeObjectURL(url);
  }
}

/**
 * @param {ComponentRuntime} runtime
 * @param {string} interfaceName
 * @param {string} functionName
 * @param {unknown[]} args
 * @param {Dispatch} dispatch
 * @returns {Promise<unknown>}
 */
export async function invoke(runtime, interfaceName, functionName, args, dispatch) {
  const instance = await runtime.namespace.instantiate(name => {
    const module = runtime.modules.get(name);
    if (!module) throw new Error(`missing compiled core module ${name}`);
    return module;
  }, makeImports(dispatch));
  const shortName = interfaceName.slice(interfaceName.lastIndexOf("/") + 1).split("@")[0];
  const exports = instance[shortName];
  const jsName = functionName.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
  if (typeof exports?.[jsName] !== "function") {
    throw new Error(`missing component export ${interfaceName}.${functionName}`);
  }
  return exports[jsName](...args);
}

/**
 * @param {Dispatch} dispatch
 * @returns {WebAssembly.Imports}
 */
function makeImports(dispatch) {
  /** @type {Map<string, WebAssembly.ModuleImports>} */
  const interfaces = new Map();
  return new Proxy(/** @type {WebAssembly.Imports} */ ({}), {
    get(_target, interfaceName) {
      if (typeof interfaceName !== "string" || interfaceName === "then") return undefined;
      let interfaceImports = interfaces.get(interfaceName);
      if (!interfaceImports) {
        interfaceImports = new Proxy(/** @type {WebAssembly.ModuleImports} */ ({}), {
          get(_interface, functionName) {
            if (typeof functionName !== "string" || functionName === "then") return undefined;
            const witName = functionName.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);
            /** @type {ComponentFunction} */
            const importedFunction = (...args) => dispatch(interfaceName, witName, args);
            return importedFunction;
          },
        });
        interfaces.set(interfaceName, interfaceImports);
      }
      return interfaceImports;
    },
  });
}
