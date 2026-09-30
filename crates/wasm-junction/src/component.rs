use std::error::Error;
use std::fmt::{self, Display};
use std::sync::Arc;

use wasmparser::{ComponentExternalKind, ComponentTypeRef, Encoding, Parser, Payload};

/// Component bytes and engine-independent interface metadata.
#[derive(Clone, Debug)]
pub struct Component {
    #[allow(dead_code, reason = "engines compile retained bytes after loading")]
    bytes: Arc<[u8]>,
    name: Option<String>,
    imports: Vec<String>,
    exports: Vec<String>,
}

impl Component {
    /// Reads and inspects a component from memory.
    ///
    /// # Errors
    ///
    /// Returns [`ComponentError`] if the bytes are malformed or encode a core module.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<Self, ComponentError> {
        let bytes = bytes.into();
        let (imports, exports) = inspect(&bytes)?;
        Ok(Self {
            bytes,
            name: None,
            imports,
            exports,
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
}

fn inspect(bytes: &[u8]) -> Result<(Vec<String>, Vec<String>), ComponentError> {
    let mut imports = Vec::new();
    let mut exports = Vec::new();
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
                        imports.push(import.name.name.to_owned());
                    }
                }
            }
            Payload::ComponentExportSection(reader) if depth == 0 => {
                for export in reader {
                    let export = export.map_err(ComponentError::Parse)?;
                    if export.kind == ComponentExternalKind::Instance {
                        exports.push(export.name.name.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    Ok((imports, exports))
}

/// A failure to inspect a [`Component`].
#[non_exhaustive]
#[derive(Debug)]
pub enum ComponentError {
    /// The binary is malformed.
    Parse(wasmparser::BinaryReaderError),
    /// The binary encodes a core WebAssembly module instead of a component.
    CoreModule,
}

impl Display for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "could not parse component: {error}"),
            Self::CoreModule => {
                formatter.write_str("expected a WebAssembly component, found a core module")
            }
        }
    }
}

impl Error for ComponentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::CoreModule => None,
        }
    }
}
