/// Errors never carry passwords or tokens.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid credentials or session")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("password hashing failed")]
    Hash,
    #[error("git error: {0}")]
    Git(String),
}

pub type Result<T> = std::result::Result<T, Error>;
