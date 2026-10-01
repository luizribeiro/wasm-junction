use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmparser::component_types::{ComponentAnyTypeId, ComponentEntityType};
use wasmparser::{ComponentExternalKind, ComponentTypeRef, Encoding, Parser, Payload, Validator};

/// Component bytes and engine-independent interface metadata.
#[derive(Clone, Debug)]
pub struct Component {
    bytes: Arc<[u8]>,
    name: Option<String>,
    imports: Vec<String>,
    type_imports: Vec<String>,
    exports: Vec<String>,
    resource_exports: Vec<String>,
    sections: HashMap<String, Range<usize>>,
}

impl Component {
    /// Reads and inspects a component from memory.
    ///
    /// # Errors
    ///
    /// Returns [`ComponentError`] if the bytes are malformed or encode a core module.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<Self, ComponentError> {
        let bytes = bytes.into();
        let metadata = inspect(&bytes)?;
        Ok(Self {
            bytes,
            name: None,
            imports: metadata.imports,
            type_imports: metadata.type_imports,
            exports: metadata.exports,
            resource_exports: metadata.resource_exports,
            sections: metadata.sections,
        })
    }

    /// Reads and inspects a component from a file, using its file stem as the name.
    ///
    /// # Errors
    ///
    /// Returns [`ComponentError`] if the file cannot be read, has no file stem, or is not a
    /// well-formed component.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ComponentError> {
        let path = path.as_ref();
        let name = path
            .file_stem()
            .ok_or_else(|| ComponentError::MissingFileStem(path.to_owned()))?
            .to_string_lossy()
            .into_owned();
        let mut component = Self::from_bytes(std::fs::read(path).map_err(ComponentError::Io)?)?;
        component.name = Some(name);
        Ok(component)
    }

    /// Replaces the application name used when this component is loaded.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Returns the application name assigned to this component, if any.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the versioned names of imported interfaces.
    #[must_use]
    pub fn imports(&self) -> &[String] {
        &self.imports
    }

    /// Returns imported interfaces that contain types but no routable functions.
    #[must_use]
    pub fn type_imports(&self) -> &[String] {
        &self.type_imports
    }

    /// Returns the versioned names of exported interfaces.
    #[must_use]
    pub fn exports(&self) -> &[String] {
        &self.exports
    }

    /// Returns the raw contents of the first custom section named `name`.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&[u8]> {
        let range = self.sections.get(name)?;
        self.bytes.get(range.clone())
    }
    pub(crate) fn into_parts(self) -> ComponentParts {
        ComponentParts {
            bytes: self.bytes,
            name: self.name,
            imports: self.imports,
            exports: self.exports,
            resource_exports: self.resource_exports,
        }
    }
}

pub(crate) struct ComponentParts {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) name: Option<String>,
    pub(crate) imports: Vec<String>,
    pub(crate) exports: Vec<String>,
    pub(crate) resource_exports: Vec<String>,
}

struct Metadata {
    imports: Vec<String>,
    type_imports: Vec<String>,
    exports: Vec<String>,
    resource_exports: Vec<String>,
    sections: HashMap<String, Range<usize>>,
}

fn inspect(bytes: &[u8]) -> Result<Metadata, ComponentError> {
    let types = Validator::new()
        .validate_all(bytes)
        .map_err(ComponentError::Parse)?;
    let mut metadata = Metadata {
        imports: Vec::new(),
        type_imports: Vec::new(),
        exports: Vec::new(),
        resource_exports: Vec::new(),
        sections: HashMap::new(),
    };
    let mut depth = 0_u32;
    let mut saw_header = false;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(ComponentError::Parse)? {
            Payload::Version { encoding, .. } if !saw_header => {
                saw_header = true;
                if encoding != Encoding::Component {
                    return Err(ComponentError::CoreModule);
                }
            }
            Payload::Version { .. } => depth += 1,
            Payload::End(_) if depth > 0 => depth -= 1,
            Payload::ComponentImportSection(reader) if depth == 0 => {
                for import in reader {
                    let import = import.map_err(ComponentError::Parse)?;
                    if let ComponentTypeRef::Instance(index) = import.ty {
                        let ComponentAnyTypeId::Instance(id) =
                            types.as_ref().component_any_type_at(index)
                        else {
                            continue;
                        };
                        let routable = types[id]
                            .exports
                            .values()
                            .any(|item| matches!(item.ty, ComponentEntityType::Func(_)));
                        let destination = if routable {
                            &mut metadata.imports
                        } else {
                            &mut metadata.type_imports
                        };
                        destination.push(import.name.name.to_owned());
                    }
                }
            }
            Payload::ComponentExportSection(reader) if depth == 0 => {
                for export in reader {
                    let export = export.map_err(ComponentError::Parse)?;
                    if export.kind == ComponentExternalKind::Instance {
                        metadata.exports.push(export.name.name.to_owned());
                        let id = types.as_ref().component_instance_at(export.index);
                        if types[id].exports.values().any(|item| {
                            matches!(
                                item.ty,
                                ComponentEntityType::Type {
                                    referenced: ComponentAnyTypeId::Resource(_),
                                    ..
                                }
                            )
                        }) {
                            metadata.resource_exports.push(export.name.name.to_owned());
                        }
                    }
                }
            }
            Payload::CustomSection(section) if depth == 0 => {
                let range = section.data_range();
                let start =
                    usize::try_from(range.start).map_err(|_| ComponentError::ComponentTooLarge)?;
                let end =
                    usize::try_from(range.end).map_err(|_| ComponentError::ComponentTooLarge)?;
                metadata
                    .sections
                    .entry(section.name().to_owned())
                    .or_insert(start..end);
            }
            _ => {}
        }
    }
    Ok(metadata)
}

/// A failure to inspect a [`Component`].
#[non_exhaustive]
#[derive(Debug)]
pub enum ComponentError {
    /// The component file could not be read.
    Io(std::io::Error),
    /// The binary is malformed.
    Parse(wasmparser::BinaryReaderError),
    /// The binary encodes a core WebAssembly module instead of a component.
    CoreModule,
    /// A section offset cannot be represented on this target.
    ComponentTooLarge,
    /// A file path did not contain a usable file stem.
    MissingFileStem(PathBuf),
}

impl Display for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not read component: {error}"),
            Self::Parse(error) => write!(formatter, "could not parse component: {error}"),
            Self::CoreModule => {
                formatter.write_str("expected a WebAssembly component, found a core module")
            }
            Self::ComponentTooLarge => {
                formatter.write_str("component is too large for this target")
            }
            Self::MissingFileStem(path) => {
                write!(
                    formatter,
                    "component path `{}` has no file stem",
                    path.display()
                )
            }
        }
    }
}

impl Error for ComponentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::CoreModule | Self::ComponentTooLarge | Self::MissingFileStem(_) => None,
        }
    }
}
