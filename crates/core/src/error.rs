use thiserror::Error;

/// Errors any provider or store can surface to the app.
#[derive(Debug, Error)]
pub enum Error {
    #[error("task {0} was not found")]
    NotFound(String),

    #[error("{0}")]
    Invalid(String),

    #[error("storage error: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, Error>;
