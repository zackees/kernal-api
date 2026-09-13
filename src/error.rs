//! Owned application error context with bounded display text.

use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter, Write as _};

/// Maximum UTF-8 bytes retained for one error context message.
pub const MAX_CONTEXT_BYTES: usize = 8 * 1024;

/// Facade-owned application error with optional source chaining.
#[derive(Debug)]
pub struct Error {
    message: String,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl Error {
    /// Construct a message-only error, truncating excessively long context.
    pub fn message(message: impl Display) -> Self {
        Self {
            message: bounded(message),
            source: None,
        }
    }

    /// Wrap a standard error with bounded caller-owned context.
    pub fn context<E>(message: impl Display, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self {
            message: bounded(message),
            source: Some(Box::new(source)),
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if formatter.alternate() {
            let mut source = self.source();
            while let Some(error) = source {
                write!(formatter, ": {error}")?;
                source = error.source();
            }
        }
        Ok(())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

/// Facade-owned application result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Add bounded context to a standard fallible result.
pub trait Context<T> {
    /// Attach a context message only if the result is an error.
    fn context(self, message: impl Display) -> Result<T>;
    /// Lazily attach a context message only if the result is an error.
    fn with_context<F, M>(self, message: F) -> Result<T>
    where
        F: FnOnce() -> M,
        M: Display;
}

impl<T, E> Context<T> for std::result::Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn context(self, message: impl Display) -> Result<T> {
        self.map_err(|source| Error::context(message, source))
    }

    fn with_context<F, M>(self, message: F) -> Result<T>
    where
        F: FnOnce() -> M,
        M: Display,
    {
        self.map_err(|source| Error::context(message(), source))
    }
}

fn bounded(message: impl Display) -> String {
    let mut output = BoundedText(String::with_capacity(MAX_CONTEXT_BYTES));
    let _ = write!(&mut output, "{message}");
    output.0
}

struct BoundedText(String);

impl fmt::Write for BoundedText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = MAX_CONTEXT_BYTES.saturating_sub(self.0.len());
        let mut end = remaining.min(text.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&text[..end]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_retains_source_and_bounds_display() {
        let error = Err::<(), _>(std::io::Error::other("source detail"))
            .context("opening test file")
            .unwrap_err();
        assert_eq!(error.to_string(), "opening test file");
        assert!(error.source().is_some());
        assert_eq!(
            Error::message("x".repeat(MAX_CONTEXT_BYTES + 1))
                .to_string()
                .len(),
            MAX_CONTEXT_BYTES
        );
        assert!(format!("{error:#}").contains("source detail"));
    }
}
