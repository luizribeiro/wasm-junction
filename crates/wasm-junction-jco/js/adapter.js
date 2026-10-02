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
 *   resources: ResourceDefinition[],
 * }} ComponentRuntime
 * @typedef {(
 *   interfaceName: string,
 *   functionName: string,
 *   args: unknown[],
 * ) => Promise<unknown>} Dispatch
 * @typedef {(interfaceName: string, resourceName: string, id: number) => Promise<void>} DropResource
 * @typedef {[string, string]} ResourceFunction
 * @typedef {[string, string, string, string | undefined, ResourceFunction[], ResourceFunction[]]} ResourceDefinition
 */

const RESOURCE_MARKER = "$wasm-junction-resource";

/**
 * @param {string} source
 * @param {string[]} names
 * @param {BufferSource[]} modules
 * @param {ResourceDefinition[]} [resources]
 * @returns {Promise<ComponentRuntime>}
 */
export async function compileComponent(source, names, modules, resources = []) {
  const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  try {
    const namespace = /** @type {ComponentNamespace} */ (await import(url));
    const compiled = await Promise.all(modules.map(module => WebAssembly.compile(module)));
    return {
      namespace,
      modules: new Map(names.map((name, index) => [name, compiled[index]])),
      resources,
    };
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
 * @param {DropResource} [dropResource]
 * @returns {Promise<unknown>}
 */
export async function invoke(
  runtime,
  interfaceName,
  functionName,
  args,
  dispatch,
  dropResource = () => Promise.reject(new Error("resource drops are unavailable")),
) {
  /** @type {Promise<void>[]} */
  const drops = [];
  const classes = makeResourceClasses(runtime.resources, dispatch, dropResource, drops);
  const instance = await runtime.namespace.instantiate(
    name => {
      const module = runtime.modules.get(name);
      if (!module) throw new Error(`missing compiled core module ${name}`);
      return module;
    },
    makeImports(dispatch, classes),
  );
  const shortName = interfaceName
    .slice(interfaceName.lastIndexOf("/") + 1)
    .split("@")[0]
    .replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
  const exports = instance[shortName];
  const jsName = functionName.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
  if (typeof exports?.[jsName] !== "function") {
    throw new Error(`missing component export ${interfaceName}.${functionName}`);
  }
  try {
    return await exports[jsName](...args.map(value => materialize(value, classes)));
  } finally {
    await Promise.all(drops);
  }
}

/**
 * @param {Dispatch} dispatch
 * @param {Map<string, Map<string, Function>>} classes
 * @returns {WebAssembly.Imports}
 */
function makeImports(dispatch, classes) {
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
            const resources = resourceClasses(classes, interfaceName);
            const resourceClass = resources?.get(functionName);
            if (resourceClass) return resourceClass;
            const witName = functionName.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);
            /** @type {ComponentFunction} */
            const importedFunction = (...args) =>
              dispatch(interfaceName, witName, args).then(
                value => materialize(value, classes),
                error => Promise.reject(materialize(error, classes)),
              );
            return importedFunction;
          },
        });
        interfaces.set(interfaceName, interfaceImports);
      }
      return interfaceImports;
    },
  });
}

/**
 * @param {ResourceDefinition[]} definitions
 * @param {Dispatch} dispatch
 * @param {DropResource} dropResource
 * @param {Promise<void>[]} drops
 */
function makeResourceClasses(definitions, dispatch, dropResource, drops) {
  /** @type {Map<string, Map<string, Function>>} */
  const interfaces = new Map();
  for (const [
    interfaceName,
    resourceName,
    className,
    constructorName,
    methods,
    statics,
  ] of definitions) {
    /** @param {...unknown} args */
    function Resource(...args) {
      if (!constructorName) throw new TypeError(`${className} has no constructor`);
      return dispatch(interfaceName, constructorName, args).then(value =>
        materialize(value, interfaces),
      );
    }
    Object.defineProperty(Resource.prototype, Symbol.dispose, {
      /** @this {{id?: number}} */
      value() {
        const id = this.id;
        if (typeof id !== "number" || !Number.isInteger(id)) {
          throw new TypeError(`${className} resource is no longer valid`);
        }
        delete this.id;
        drops.push(dropResource(interfaceName, resourceName, id));
      },
    });
    Object.defineProperty(Resource, "name", { value: className });
    for (const [witName, jsName] of methods) {
      /** @this {unknown} @param {...unknown} args */
      const method = function (...args) {
        return dispatch(interfaceName, witName, [this, ...args]).then(value =>
          materialize(value, interfaces),
        );
      };
      Object.defineProperty(Resource.prototype, jsName, {
        value: method,
      });
    }
    for (const [witName, jsName] of statics) {
      /** @type {ComponentFunction} */
      const staticFunction = (...args) =>
        dispatch(interfaceName, witName, args).then(value => materialize(value, interfaces));
      Object.defineProperty(Resource, jsName, {
        value: staticFunction,
      });
    }
    let resources = interfaces.get(interfaceName);
    if (!resources) {
      resources = new Map();
      interfaces.set(interfaceName, resources);
    }
    resources.set(className, Resource);
    resources.set(resourceName, Resource);
  }
  return interfaces;
}

/** @param {Map<string, Map<string, Function>>} classes @param {string} interfaceName */
function resourceClasses(classes, interfaceName) {
  const exact = classes.get(interfaceName);
  if (exact) return exact;
  for (const [candidate, resources] of classes) {
    if (candidate.split("@")[0] === interfaceName) return resources;
  }
}

/** @param {unknown} value @param {Map<string, Map<string, Function>>} classes @returns {any} */
function materialize(value, classes) {
  if (!value || typeof value !== "object" || value instanceof Uint8Array) return value;
  if (Array.isArray(value)) return value.map(item => materialize(item, classes));
  const object = /** @type {Record<string, any>} */ (value);
  if (RESOURCE_MARKER in object) {
    const [interfaceName, resourceName, id] = object[RESOURCE_MARKER];
    const Resource = resourceClasses(classes, interfaceName)?.get(resourceName);
    if (!Resource) throw new TypeError(`unknown resource ${interfaceName}/${resourceName}`);
    const resource = Object.create(Resource.prototype);
    resource.id = id;
    return resource;
  }
  for (const key of Object.keys(object)) object[key] = materialize(object[key], classes);
  return object;
}
