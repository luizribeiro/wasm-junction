use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display};
use std::panic::Location;
use std::sync::Arc;

use crate::middleware::ErasedMiddleware;
use crate::{Engine, Middleware, Provided, Provider};

/// An application assembled from host providers, middleware, and WebAssembly components.
pub struct App(#[allow(dead_code)] pub(crate) Arc<AppInner>);

pub(crate) struct AppInner {
    #[allow(dead_code)]
    pub(crate) engine: Arc<dyn Engine>,
    #[allow(dead_code)]
    pub(crate) providers: HashMap<&'static str, Arc<dyn Provider>>,
    #[allow(dead_code)]
    pub(crate) middleware: Arc<[Arc<dyn ErasedMiddleware>]>,
}

impl App {
    /// Starts configuring an application.
    #[must_use]
    pub fn builder() -> AppBuilder {
        AppBuilder::default()
    }
}

struct ProviderRegistration {
    provided: Provided,
    location: &'static Location<'static>,
}

/// Incrementally configures an [`App`].
#[derive(Default)]
pub struct AppBuilder {
    engine: Option<Arc<dyn Engine>>,
    providers: Vec<ProviderRegistration>,
    middleware: Vec<Arc<dyn ErasedMiddleware>>,
}

impl AppBuilder {
    /// Selects the engine used to compile subsequently loaded components.
    #[must_use]
    pub fn engine(mut self, engine: impl Engine + 'static) -> Self {
        self.engine = Some(Arc::new(engine));
        self
    }

    /// Registers one host interface provider.
    #[track_caller]
    #[must_use]
    pub fn provide(mut self, provided: Provided) -> Self {
        self.providers.push(ProviderRegistration {
            provided,
            location: Location::caller(),
        });
        self
    }

    /// Appends middleware; the first registered middleware runs outermost.
    #[must_use]
    pub fn middleware(mut self, middleware: impl Middleware + 'static) -> Self {
        self.middleware.push(Arc::new(middleware));
        self
    }

    /// Applies a reusable builder transformation.
    #[must_use]
    pub fn apply(self, transform: impl FnOnce(Self) -> Self) -> Self {
        transform(self)
    }

    /// Validates the configuration and creates an application.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if no engine was selected or an interface has multiple providers.
    pub fn build(self) -> Result<App, BuildError> {
        let engine = self.engine.ok_or(BuildError::MissingEngine)?;
        let mut providers = HashMap::new();
        let mut locations = HashMap::new();
        for registration in self.providers {
            let (interface, provider) = registration.provided.into_parts();
            if let Some(first) = locations.insert(interface, registration.location) {
                return Err(BuildError::DuplicateProvider {
                    interface,
                    first,
                    second: registration.location,
                });
            }
            providers.insert(interface, provider);
        }
        Ok(App(Arc::new(AppInner {
            engine,
            providers,
            middleware: self.middleware.into(),
        })))
    }
}

/// A configuration error found while building an [`App`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// No engine was selected.
    MissingEngine,
    /// More than one provider was registered for an interface.
    DuplicateProvider {
        /// The duplicated interface.
        interface: &'static str,
        /// The first registration site.
        first: &'static Location<'static>,
        /// The second registration site.
        second: &'static Location<'static>,
    },
}

impl Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEngine => formatter.write_str("an engine is required"),
            Self::DuplicateProvider {
                interface,
                first,
                second,
            } => write!(
                formatter,
                "provider for `{interface}` registered at {first} and again at {second}"
            ),
        }
    }
}

impl Error for BuildError {}
