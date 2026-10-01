use std::collections::BTreeMap;

use wit_component::{DecodedWasm, decode};
use wit_parser::{Function, Resolve, Type, TypeDefKind, WorldItem};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ValueType {
    Bool,
    S8,
    U8,
    S16,
    U16,
    S32,
    U32,
    S64,
    U64,
    String,
    Unsupported(&'static str),
}

impl ValueType {
    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::S8 => "s8",
            Self::U8 => "u8",
            Self::S16 => "s16",
            Self::U16 => "u16",
            Self::S32 => "s32",
            Self::U32 => "u32",
            Self::S64 => "s64",
            Self::U64 => "u64",
            Self::String => "string",
            Self::Unsupported(name) => name,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionType {
    pub(crate) params: Vec<ValueType>,
    pub(crate) result: Option<ValueType>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Signatures {
    #[cfg(any(test, target_family = "wasm"))]
    imports: BTreeMap<(String, String), FunctionType>,
    #[cfg(any(test, target_family = "wasm"))]
    import_aliases: BTreeMap<String, Option<String>>,
    exports: BTreeMap<(String, String), FunctionType>,
}

impl Signatures {
    pub(crate) fn from_component(bytes: &[u8]) -> Result<Self, String> {
        let DecodedWasm::Component(resolve, world) = decode(bytes)
            .map_err(|error| format!("could not inspect WebAssembly component: {error:#}"))?
        else {
            return Err("expected a WebAssembly component, found a WIT package".to_owned());
        };
        #[cfg(any(test, target_family = "wasm"))]
        let imports = collect(&resolve, resolve.worlds[world].imports.iter());
        Ok(Self {
            #[cfg(any(test, target_family = "wasm"))]
            import_aliases: import_aliases(&imports),
            #[cfg(any(test, target_family = "wasm"))]
            imports,
            exports: collect(&resolve, resolve.worlds[world].exports.iter()),
        })
    }

    #[cfg(any(test, target_family = "wasm"))]
    pub(crate) fn import(
        &self,
        interface: &str,
        function: &str,
    ) -> Result<(&str, &FunctionType), String> {
        let key = (interface.to_owned(), function.to_owned());
        if let Some(((resolved, _), signature)) = self.imports.get_key_value(&key) {
            return Ok((resolved, signature));
        }
        let Some(resolved) = self.import_aliases.get(interface) else {
            return Err(format!("unknown component import `{interface}.{function}`"));
        };
        let Some(resolved) = resolved else {
            return Err(format!(
                "ambiguous component import interface alias `{interface}`"
            ));
        };
        self.imports
            .get_key_value(&(resolved.clone(), function.to_owned()))
            .map(|((resolved, _), signature)| (resolved.as_str(), signature))
            .ok_or_else(|| format!("unknown component import `{interface}.{function}`"))
    }

    pub(crate) fn export(&self, interface: &str, function: &str) -> Option<&FunctionType> {
        self.exports
            .get(&(interface.to_owned(), function.to_owned()))
    }
}

#[cfg(any(test, target_family = "wasm"))]
fn import_aliases(
    imports: &BTreeMap<(String, String), FunctionType>,
) -> BTreeMap<String, Option<String>> {
    let mut aliases: BTreeMap<String, Option<String>> = BTreeMap::new();
    for interface in imports.keys().map(|(interface, _)| interface) {
        let Some((base, _)) = interface.split_once('@') else {
            continue;
        };
        aliases
            .entry(base.to_owned())
            .and_modify(|resolved| {
                if resolved.as_ref() != Some(interface) {
                    *resolved = None;
                }
            })
            .or_insert_with(|| Some(interface.clone()));
    }
    aliases
}

fn collect<'a>(
    resolve: &Resolve,
    items: impl Iterator<Item = (&'a wit_parser::WorldKey, &'a WorldItem)>,
) -> BTreeMap<(String, String), FunctionType> {
    let mut signatures = BTreeMap::new();
    for (key, item) in items {
        let WorldItem::Interface { id, .. } = item else {
            continue;
        };
        let interface = resolve.name_world_key(key);
        for (name, function) in &resolve.interfaces[*id].functions {
            signatures.insert(
                (interface.clone(), name.clone()),
                function_type(resolve, function),
            );
        }
    }
    signatures
}

fn function_type(resolve: &Resolve, function: &Function) -> FunctionType {
    FunctionType {
        params: function
            .params
            .iter()
            .map(|param| value_type(resolve, param.ty))
            .collect(),
        result: function.result.map(|ty| value_type(resolve, ty)),
    }
}

fn value_type(resolve: &Resolve, ty: Type) -> ValueType {
    match ty {
        Type::Bool => ValueType::Bool,
        Type::S8 => ValueType::S8,
        Type::U8 => ValueType::U8,
        Type::S16 => ValueType::S16,
        Type::U16 => ValueType::U16,
        Type::S32 => ValueType::S32,
        Type::U32 => ValueType::U32,
        Type::S64 => ValueType::S64,
        Type::U64 => ValueType::U64,
        Type::String => ValueType::String,
        Type::Id(id) => match resolve.types[id].kind {
            TypeDefKind::Type(ty) => value_type(resolve, ty),
            ref kind => ValueType::Unsupported(kind.as_str()),
        },
        Type::F32 => ValueType::Unsupported("f32"),
        Type::F64 => ValueType::Unsupported("f64"),
        Type::Char => ValueType::Unsupported("char"),
        Type::ErrorContext => ValueType::Unsupported("error-context"),
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    fn refuses_an_ambiguous_unversioned_import_alias() {
        let imports = BTreeMap::from([
            (
                ("example:notes/api@1.0.0".to_owned(), "read".to_owned()),
                FunctionType {
                    params: Vec::new(),
                    result: None,
                },
            ),
            (
                ("example:notes/api@2.0.0".to_owned(), "read".to_owned()),
                FunctionType {
                    params: Vec::new(),
                    result: None,
                },
            ),
        ]);
        let signatures = Signatures {
            import_aliases: import_aliases(&imports),
            imports,
            exports: BTreeMap::new(),
        };

        let error = signatures.import("example:notes/api", "read").unwrap_err();
        assert_eq!(
            error,
            "ambiguous component import interface alias `example:notes/api`"
        );
        assert!(signatures.import("example:notes/api@1.0.0", "read").is_ok());
        assert!(signatures.import("example:notes/api@2.0.0", "read").is_ok());
    }
}
