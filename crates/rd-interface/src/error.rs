use std::{error::Error as StdError, fmt::Display, io};
use thiserror::Error;

#[derive(Debug, Error)]
pub struct ErrorWithContext {
    context: String,
    #[source]
    error: Box<dyn StdError + Send + Sync + 'static>,
}

impl Display for ErrorWithContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\nCaused by:\n", self.context)?;

        write!(f, "{}", self.error)?;
        Ok(())
    }
}

/// Errors in this crate.
#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0:?}")]
    IO(#[from] io::Error),
    #[error("Not matched")]
    NotMatched,
    #[error("Not enabled in config")]
    NotEnabled,
    #[error("Not implemented")]
    NotImplemented,
    #[error("Config error: {0}")]
    Config(#[from] serde_json::Error),
    #[error("Aborted by user")]
    AbortedByUser,
    #[error("Context error: {0:?}")]
    Context(#[from] crate::context::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("{0:?}")]
    Other(Box<dyn StdError + Send + Sync + 'static>),
    #[error("{0}")]
    WithContext(ErrorWithContext),
    #[error("Operation timeout: {0:?}")]
    Timeout(#[from] tokio::time::error::Elapsed),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;
pub const NOT_IMPLEMENTED: Error = Error::NotImplemented;

pub fn map_other(e: impl StdError + Send + Sync + 'static) -> Error {
    Error::Other(e.into())
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::IO(e) => e,
            e => io::Error::new(io::ErrorKind::Other, e),
        }
    }
}

impl Error {
    pub fn is_aborted(&self) -> bool {
        matches!(self, Error::AbortedByUser)
    }
    pub fn is_addr_in_use(&self) -> bool {
        match self {
            Error::IO(e) => e.kind() == io::ErrorKind::AddrInUse,
            _ => false,
        }
    }
    pub fn other(string: impl Into<String>) -> Error {
        Error::Other(string.into().into())
    }
    pub fn to_io_err(self) -> io::Error {
        match self {
            Self::IO(e) => e,
            e => io::Error::new(io::ErrorKind::Other, e),
        }
    }
}

pub trait ErrorContext<T, E> {
    fn context<C>(self, context: C) -> Result<T, Error>
    where
        C: Display + Send + Sync + 'static;
    fn with_context<C, F>(self, f: F) -> Result<T, Error>
    where
        C: Display + Send + Sync + 'static,
        F: FnOnce() -> C;
}

impl<T, E> ErrorContext<T, E> for Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn context<C>(self, context: C) -> Result<T, Error>
    where
        C: Display + Send + Sync + 'static,
    {
        self.map_err(|error| {
            Error::WithContext(ErrorWithContext {
                context: context.to_string(),
                error: Box::new(error),
            })
        })
    }
    fn with_context<C, F>(self, f: F) -> Result<T, Error>
    where
        C: Display + Send + Sync + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|error| {
            Error::WithContext(ErrorWithContext {
                context: f().to_string(),
                error: Box::new(error),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context() {
        let error = Result::<()>::Err(Error::AbortedByUser);
        let error = error.context("A context");
        assert_eq!(
            error.unwrap_err().to_string(),
            "A context\nCaused by:\nAborted by user"
        );

        let error = Result::<()>::Err(Error::AbortedByUser);
        let error = error.with_context(|| "A context");
        assert_eq!(
            error.unwrap_err().to_string(),
            "A context\nCaused by:\nAborted by user"
        );
    }

    #[test]
    fn test_nested_context() {
        let error = Result::<()>::Err(Error::AbortedByUser);
        let error = error.context("Error 1");
        let error = error.context("Error 2");
        let error = error.context("Error 3");
        assert_eq!(
            format!("{}", error.unwrap_err()),
            "Error 3\nCaused by:\nError 2\nCaused by:\nError 1\nCaused by:\nAborted by user"
        );
    }

    #[test]
    fn test_error_methods() {
        let error = Error::AbortedByUser;
        assert!(error.is_aborted());

        let error = Error::from(io::Error::new(io::ErrorKind::AddrInUse, ""));
        assert!(error.is_addr_in_use());

        let error = Error::other("Other error");
        assert_eq!(error.to_string(), "\"Other error\"");
    }
}
