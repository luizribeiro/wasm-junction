use std::collections::BTreeMap;

use wasm_junction_core::ResourceOwnership;
use wit_component::{DecodedWasm, decode};
use wit_parser::{Function, Handle, Resolve, Type, TypeDefKind, TypeOwner, WorldItem};

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
    F32,
    F64,
    Char,
    String,
    List(Box<ValueType>),
    Tuple(Vec<ValueType>),
    Record(Vec<FieldType>),
    Variant(Vec<CaseType>),
    Enum(Vec<String>),
    Flags(Vec<String>),
    Option(Box<ValueType>),
    Result {
        ok: Option<Box<ValueType>>,
        err: Option<Box<ValueType>>,
    },
    Resource(ResourceType),
    Unsupported(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceType {
    pub(crate) interface: String,
    pub(crate) name: String,
    pub(crate) ownership: ResourceOwnership,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FieldType {
    pub(crate) name: String,
    pub(crate) js_name: String,
    pub(crate) ty: ValueType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaseType {
    pub(crate) name: String,
    pub(crate) ty: Option<ValueType>,
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
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Char => "char",
            Self::String => "string",
            Self::List(_) => "list",
            Self::Tuple(_) => "tuple",
            Self::Record(_) => "record",
            Self::Variant(_) => "variant",
            Self::Enum(_) => "enum",
            Self::Flags(_) => "flags",
            Self::Option(_) => "option",
            Self::Result { .. } => "result",
            Self::Resource(resource) => match resource.ownership {
                ResourceOwnership::Own => "own",
                ResourceOwnership::Borrow => "borrow",
            },
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
        Type::Id(id) => match &resolve.types[id].kind {
            TypeDefKind::Type(ty) => value_type(resolve, *ty),
            TypeDefKind::List(ty) => ValueType::List(Box::new(value_type(resolve, *ty))),
            TypeDefKind::Tuple(tuple) => ValueType::Tuple(
                tuple
                    .types
                    .iter()
                    .map(|ty| value_type(resolve, *ty))
                    .collect(),
            ),
            TypeDefKind::Record(record) => ValueType::Record(
                record
                    .fields
                    .iter()
                    .map(|field| FieldType {
                        name: field.name.clone(),
                        js_name: js_name(&field.name),
                        ty: value_type(resolve, field.ty),
                    })
                    .collect(),
            ),
            TypeDefKind::Variant(variant) => ValueType::Variant(
                variant
                    .cases
                    .iter()
                    .map(|case| CaseType {
                        name: case.name.clone(),
                        ty: case.ty.map(|ty| value_type(resolve, ty)),
                    })
                    .collect(),
            ),
            TypeDefKind::Enum(value) => {
                ValueType::Enum(value.cases.iter().map(|case| case.name.clone()).collect())
            }
            TypeDefKind::Flags(value) => {
                ValueType::Flags(value.flags.iter().map(|flag| flag.name.clone()).collect())
            }
            TypeDefKind::Option(ty) => ValueType::Option(Box::new(value_type(resolve, *ty))),
            TypeDefKind::Result(result) => ValueType::Result {
                ok: result.ok.map(|ty| Box::new(value_type(resolve, ty))),
                err: result.err.map(|ty| Box::new(value_type(resolve, ty))),
            },
            TypeDefKind::Handle(handle) => resource_type(resolve, *handle),
            kind => ValueType::Unsupported(kind.as_str()),
        },
        Type::F32 => ValueType::F32,
        Type::F64 => ValueType::F64,
        Type::Char => ValueType::Char,
        Type::ErrorContext => ValueType::Unsupported("error-context"),
    }
}

fn resource_type(resolve: &Resolve, handle: Handle) -> ValueType {
    let (resource, ownership) = match handle {
        Handle::Own(resource) => (resource, ResourceOwnership::Own),
        Handle::Borrow(resource) => (resource, ResourceOwnership::Borrow),
    };
    let definition = &resolve.types[resource];
    let TypeOwner::Interface(owner) = definition.owner else {
        return ValueType::Unsupported("resource");
    };
    match (resolve.id_of(owner), definition.name.clone()) {
        (Some(interface), Some(name)) => ValueType::Resource(ResourceType {
            interface,
            name,
            ownership,
        }),
        _ => ValueType::Unsupported("resource"),
    }
}

pub(crate) fn js_name(name: &str) -> String {
    let mut uppercase = false;
    name.chars()
        .filter_map(|character| {
            if character == '-' {
                uppercase = true;
                None
            } else if uppercase {
                uppercase = false;
                Some(character.to_ascii_uppercase())
            } else {
                Some(character)
            }
        })
        .collect()
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    fn collects_every_plain_fixture_type() {
        let signatures =
            Signatures::from_component(wasm_junction_conformance::component()).unwrap();
        let echo = signatures
            .export(wasm_junction_conformance::SUMMARIZER, "echo")
            .unwrap();
        let ValueType::Record(fields) = &echo.params[0] else {
            panic!("echo did not accept the note record")
        };
        assert_eq!(fields[2].js_name, "signed8");
        assert!(matches!(fields[10].ty, ValueType::F32));
        assert!(matches!(fields[11].ty, ValueType::F64));
        assert!(matches!(fields[12].ty, ValueType::Char));
        assert!(matches!(fields[13].ty, ValueType::List(_)));
        assert!(matches!(fields[14].ty, ValueType::Tuple(_)));
        assert!(matches!(fields[15].ty, ValueType::Variant(_)));
        assert!(matches!(fields[16].ty, ValueType::Enum(_)));
        assert!(matches!(fields[17].ty, ValueType::Flags(_)));
        assert!(matches!(fields[18].ty, ValueType::Option(_)));
        assert!(matches!(fields[19].ty, ValueType::Result { .. }));
        assert_eq!(echo.result.as_ref(), echo.params.first());
    }

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

    #[test]
    fn collects_resource_ownership() {
        let signatures =
            Signatures::from_component(wasm_junction_conformance::resource_component()).unwrap();
        let constructor = signatures
            .import("example:resources/host@1.0.0", "[constructor]session")
            .unwrap()
            .1;
        let Some(ValueType::Resource(resource)) = &constructor.result else {
            panic!("constructor did not return a resource")
        };
        assert_eq!(resource.interface, "example:resources/host@1.0.0");
        assert_eq!(resource.name, "session");
        assert_eq!(resource.ownership, ResourceOwnership::Own);
    }
}
