/// Errors produced by the backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Git(#[from] gix::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("`git {command}` failed:\n{stderr}")]
    Command { command: String, stderr: String },
    #[error("{0}")]
    Message(String),
}

impl Error {
    pub(crate) fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

/// Converts any gitoxide sub-crate error into ours without a dedicated variant per error type.
pub(crate) trait GixResultExt<T> {
    fn git(self) -> Result<T>;
}

impl<T, E: std::error::Error + Send + Sync + 'static> GixResultExt<T> for std::result::Result<T, E> {
    fn git(self) -> Result<T> {
        self.map_err(|err| Error::Git(gix::Error::from_error(err)))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
