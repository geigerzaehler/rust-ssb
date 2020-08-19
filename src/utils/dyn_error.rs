//! Provides [DynError].

#[derive(Debug)]
/// Wrap `Box<dyn std::error::Error>` and provide a [std::error::Error] implementation.
pub struct DynError(Box<dyn std::error::Error + Send + Sync>);

impl DynError {
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for DynError {
    fn from(value: Box<dyn std::error::Error + Send + Sync>) -> Self {
        DynError(value)
    }
}

impl std::fmt::Display for DynError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for DynError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }

    fn description(&self) -> &str {
        #[allow(deprecated)]
        self.0.description()
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        self.source()
    }
}
