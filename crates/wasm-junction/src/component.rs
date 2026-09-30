use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display};
use std::ops::Range;
use std::sync::Arc;

use wasmparser::{ComponentExternalKind, ComponentTypeRef, Encoding, Parser, Payload};

/// Component bytes and engine-independent interface metadata.
#[derive(Clone, Debug)]
pub struct Component {
    bytes: Arc<[u8]>,
    name: Option<String>,
    imports: Vec<String>,
    exports: Vec<String>,
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
            exports: metadata.exports,
            sections: metadata.sections,
        })
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
}

struct Metadata {
    imports: Vec<String>,
    exports: Vec<String>,
    sections: HashMap<String, Range<usize>>,
}

fn inspect(bytes: &[u8]) -> Result<Metadata, ComponentError> {
    let mut metadata = Metadata {
        imports: Vec::new(),
        exports: Vec::new(),
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
                    if matches!(import.ty, ComponentTypeRef::Instance(_)) {
                        metadata.imports.push(import.name.name.to_owned());
                    }
                }
            }
            Payload::ComponentExportSection(reader) if depth == 0 => {
                for export in reader {
                    let export = export.map_err(ComponentError::Parse)?;
                    if export.kind == ComponentExternalKind::Instance {
                        metadata.exports.push(export.name.name.to_owned());
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
    /// The binary is malformed.
    Parse(wasmparser::BinaryReaderError),
    /// The binary encodes a core WebAssembly module instead of a component.
    CoreModule,
    /// A section offset cannot be represented on this target.
    ComponentTooLarge,
}

impl Display for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "could not parse component: {error}"),
            Self::CoreModule => {
                formatter.write_str("expected a WebAssembly component, found a core module")
            }
            Self::ComponentTooLarge => {
                formatter.write_str("component is too large for this target")
            }
        }
    }
}

impl Error for ComponentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::CoreModule | Self::ComponentTooLarge => None,
        }
    }
}
