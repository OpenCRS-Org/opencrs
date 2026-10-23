use std::fmt;

/// Everything that can go wrong in the core, as text a CLI can print.
#[derive(Debug)]
pub enum Error {
    /// The input is not readable JSON.
    Json(serde_json::Error),
    /// Any other failure, with the message Go's validator gives.
    Message(String),
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Message(s.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(e) => e.fmt(f),
            Error::Message(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Json(e) => Some(e),
            Error::Message(_) => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
