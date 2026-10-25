use std::fmt;

/// Errors of the U-BOT core; callers map them to command errors without panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UbotError {
    BadInput(String),
    Limit(String),
    UnknownProduct(String),
    Geometry(String),
}

impl fmt::Display for UbotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadInput(m) => write!(f, "bad input: {m}"),
            Self::Limit(m) => write!(f, "limit exceeded: {m}"),
            Self::UnknownProduct(id) => write!(f, "unknown product: {id}"),
            Self::Geometry(m) => write!(f, "geometry error: {m}"),
        }
    }
}

impl std::error::Error for UbotError {}
