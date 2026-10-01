export async function compileComponent(source, names, modules) {
  const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  try {
    const namespace = await import(url);
    const compiled = await Promise.all(modules.map(module => WebAssembly.compile(module)));
    return { namespace, modules: new Map(names.map((name, index) => [name, compiled[index]])) };
  } finally {
    URL.revokeObjectURL(url);
  }
}

export async function invoke(runtime, interfaceName, functionName, args, dispatch) {
  const instance = await runtime.namespace.instantiate(
    name => {
      const module = runtime.modules.get(name);
      if (!module) throw new Error(`missing compiled core module ${name}`);
      return module;
    },
    makeImports(dispatch),
  );
  const shortName = interfaceName.slice(interfaceName.lastIndexOf("/") + 1).split("@")[0];
  const exports = instance[shortName];
  const jsName = functionName.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
  if (typeof exports?.[jsName] !== "function") {
    throw new Error(`missing component export ${interfaceName}.${functionName}`);
  }
  return exports[jsName](...args);
}

function makeImports(dispatch) {
  const interfaces = new Map();
  return new Proxy({}, {
    get(_target, interfaceName) {
      if (typeof interfaceName !== "string" || interfaceName === "then") return undefined;
      if (!interfaces.has(interfaceName)) {
        interfaces.set(interfaceName, new Proxy({}, {
          get(_interface, functionName) {
            if (typeof functionName !== "string" || functionName === "then") return undefined;
            const witName = functionName.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);
            return (...args) => dispatch(interfaceName, witName, args);
          },
        }));
      }
      return interfaces.get(interfaceName);
    },
  });
}
