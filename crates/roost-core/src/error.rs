use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(String),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("hash mismatch for {file}")]
    HashMismatch { file: String },
    #[error("insufficient disk space: need {needed} bytes, {available} available")]
    InsufficientSpace { needed: u64, available: u64 },
    #[error("Ollama is unreachable")]
    OllamaUnreachable,
    #[error("gated repository: {0}")]
    GatedRepo(String),
    #[error("cancelled")]
    Cancelled,
    #[error("I/O error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("a model named {0} already exists in Ollama: choose another name or delete it first in Installed")]
    NameCollision(String),
    #[error("invalid model name: {0}")]
    InvalidName(String),
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    #[error("Ollama error: {0}")]
    Ollama(String),
}

pub type Result<T> = std::result::Result<T, Error>;
