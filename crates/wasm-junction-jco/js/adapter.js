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
 * @typedef {(id: bigint) => Promise<Uint8Array | unknown[] | null>} ReadStream
 * @typedef {(id: bigint) => Promise<void>} CloseStream
 * @typedef {{ read: ReadStream, close: CloseStream }} StreamFunctions
 * @typedef {[string, string]} ResourceFunction
 * @typedef {[string, string, string, string | undefined, ResourceFunction[], ResourceFunction[]]} ResourceDefinition
 * @typedef {{ poisoned: boolean }} ImportState
 */

const RESOURCE_MARKER = "$wasm-junction-resource";
const STREAM_MARKER = "$wasm-junction-stream";
const STREAM_ID = Symbol.for("wasm-junction:stream-id");
const IMPORT_FAILURE = Symbol("wasm-junction-import-failure");
const IMPORT_STATE = "wasm-junction:internal/import-state";

/** @param {unknown} value @returns {object} */
export function poison(value) {
  return { [IMPORT_FAILURE]: value };
}

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
 * @param {ReadStream} [readStream]
 * @param {CloseStream} [closeStream]
 * @returns {Promise<unknown>}
 */
export async function invoke(
  runtime,
  interfaceName,
  functionName,
  args,
  dispatch,
  dropResource = () => Promise.reject(new Error("resource drops are unavailable")),
  readStream = () => Promise.reject(new Error("stream reads are unavailable")),
  closeStream = () => Promise.reject(new Error("stream closes are unavailable")),
) {
  /** @type {Promise<void>[]} */
  const drops = [];
  const state = { poisoned: false };
  const streams = { read: readStream, close: closeStream };
  const classes = makeResourceClasses(
    runtime.resources,
    dispatch,
    dropResource,
    drops,
    state,
    streams,
  );
  const instance = await runtime.namespace.instantiate(
    name => {
      const module = runtime.modules.get(name);
      if (!module) throw new Error(`missing compiled core module ${name}`);
      return module;
    },
    makeImports(dispatch, classes, state, streams),
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
    return dematerialize(
      await exports[jsName](
        ...args.map(value => materialize(value, classes, state, readStream, closeStream)),
      ),
    );
  } finally {
    await Promise.all(drops);
  }
}

/**
 * @param {Dispatch} dispatch
 * @param {Map<string, Map<string, Function>>} classes
 * @param {ImportState} state
 * @param {StreamFunctions} streams
 * @returns {WebAssembly.Imports}
 */
function makeImports(dispatch, classes, state, streams) {
  /** @type {Map<string, WebAssembly.ModuleImports>} */
  const interfaces = new Map();
  return new Proxy(/** @type {WebAssembly.Imports} */ ({}), {
    get(_target, interfaceName) {
      if (typeof interfaceName !== "string" || interfaceName === "then") return undefined;
      if (interfaceName === IMPORT_STATE) return state;
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
              callImport(state, dispatch, classes, interfaceName, witName, args, streams);
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
 * @param {ImportState} state
 * @param {StreamFunctions} streams
 */
function makeResourceClasses(definitions, dispatch, dropResource, drops, state, streams) {
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
      return callImport(state, dispatch, interfaces, interfaceName, constructorName, args, streams);
    }
    Object.defineProperty(Resource.prototype, Symbol.dispose, {
      /** @this {{id?: number}} */
      value() {
        if (state.poisoned) return;
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
        return callImport(
          state,
          dispatch,
          interfaces,
          interfaceName,
          witName,
          [this, ...args],
          streams,
        );
      };
      Object.defineProperty(Resource.prototype, jsName, {
        value: method,
      });
    }
    for (const [witName, jsName] of statics) {
      /** @type {ComponentFunction} */
      const staticFunction = (...args) =>
        callImport(state, dispatch, interfaces, interfaceName, witName, args, streams);
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

/**
 * @param {ImportState} state
 * @param {Dispatch} dispatch
 * @param {Map<string, Map<string, Function>>} classes
 * @param {string} interfaceName
 * @param {string} functionName
 * @param {unknown[]} args
 * @param {StreamFunctions} streams
 * @returns {Promise<unknown>}
 */
function callImport(state, dispatch, classes, interfaceName, functionName, args, streams) {
  return dispatch(
    interfaceName,
    functionName,
    args.map(value => dematerialize(value)),
  ).then(
    value => {
      if (value && typeof value === "object" && IMPORT_FAILURE in value) {
        state.poisoned = true;
        return materialize(value[IMPORT_FAILURE], classes, state, streams.read, streams.close);
      }
      return materialize(value, classes, state, streams.read, streams.close);
    },
    error => Promise.reject(materialize(error, classes, state, streams.read, streams.close)),
  );
}

/** @param {Map<string, Map<string, Function>>} classes @param {string} interfaceName */
function resourceClasses(classes, interfaceName) {
  const exact = classes.get(interfaceName);
  if (exact) return exact;
  for (const [candidate, resources] of classes) {
    if (candidate.split("@")[0] === interfaceName) return resources;
  }
}

/**
 * @param {unknown} value
 * @param {Map<string, Map<string, Function>>} classes
 * @param {ImportState} state
 * @param {ReadStream} readStream
 * @param {CloseStream} closeStream
 * @returns {any}
 */
function materialize(value, classes, state, readStream, closeStream) {
  if (!value || typeof value !== "object" || value instanceof Uint8Array) return value;
  if (Array.isArray(value)) {
    return value.map(item => materialize(item, classes, state, readStream, closeStream));
  }
  const object = /** @type {Record<string, any>} */ (value);
  if (STREAM_MARKER in object) {
    const [kind, id] = object[STREAM_MARKER];
    if (kind !== "host") throw new TypeError(`unexpected ${kind} stream from Rust`);
    return hostStream(id, classes, state, readStream, closeStream);
  }
  if (RESOURCE_MARKER in object) {
    const [interfaceName, resourceName, id] = object[RESOURCE_MARKER];
    const Resource = resourceClasses(classes, interfaceName)?.get(resourceName);
    if (!Resource) throw new TypeError(`unknown resource ${interfaceName}/${resourceName}`);
    const resource = Object.create(Resource.prototype);
    resource.id = id;
    return resource;
  }
  for (const key of Object.keys(object)) {
    object[key] = materialize(object[key], classes, state, readStream, closeStream);
  }
  return object;
}

/**
 * @param {bigint} id
 * @param {Map<string, Map<string, Function>>} classes
 * @param {ImportState} state
 * @param {ReadStream} readStream
 * @param {CloseStream} closeStream
 * @returns {any}
 */
function hostStream(id, classes, state, readStream, closeStream) {
  /** @type {any[]} */
  let pending = [];
  let ended = false;
  const iterator = {
    async next() {
      if (pending.length > 0) return { done: false, value: pending.shift() };
      if (state.poisoned) throw new WebAssembly.RuntimeError("stream read after import failure");
      const chunk = await readStream(id);
      if (chunk === null) {
        ended = true;
        return { done: true, value: undefined };
      }
      pending =
        chunk instanceof Uint8Array
          ? [...chunk]
          : chunk.map(item => materialize(item, classes, state, readStream, closeStream));
      return { done: false, value: pending.shift() };
    },
    async return() {
      if (!ended) {
        ended = true;
        pending = [];
        await closeStream(id);
      }
      return { done: true, value: undefined };
    },
    [Symbol.asyncIterator]() {
      return this;
    },
  };
  return {
    [STREAM_ID]: id,
    [Symbol.asyncIterator]() {
      return iterator;
    },
  };
}

/** @param {any} value @returns {any} */
function dematerialize(value) {
  if (!value || typeof value !== "object" || value instanceof Uint8Array) return value;
  if (Symbol.asyncIterator in value) {
    const id = value[STREAM_ID];
    return id === undefined ? value : { [STREAM_MARKER]: ["host", id] };
  }
  if (Array.isArray(value)) return value.map(item => dematerialize(item));
  for (const key of Object.keys(value)) value[key] = dematerialize(value[key]);
  return value;
}

/** @param {any} stream @param {boolean} byteStream @returns {Promise<any[] | Uint8Array | null>} */
export async function readGuestStream(stream, byteStream) {
  const bulk = typeof stream.read === "function";
  const item = bulk
    ? await stream.read({ count: 65536 })
    : await stream[Symbol.asyncIterator]().next();
  if (item.done) return null;
  if (!byteStream) return bulk ? item.value : [item.value];
  if (item.value instanceof Uint8Array) return item.value;
  if (typeof item.value === "number") return Uint8Array.of(item.value);
  return Uint8Array.from(item.value);
}

/** @param {any} stream @returns {Promise<void>} */
export async function closeGuestStream(stream) {
  if (typeof stream.return === "function") await stream.return();
  else stream[Symbol.dispose]?.();
}
