//! Errors that carry one of the shared error codes (PLAN section 5.10).

/// An error with a machine-readable code and a plain-English message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct CodedError {
    /// One of the shared codes, such as `vault_empty`.
    pub code: &'static str,
    /// What to tell the user.
    pub message: String,
}

/// Builds an [`anyhow::Error`] carrying a [`CodedError`].
pub fn coded(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(CodedError {
        code,
        message: message.into(),
    })
}
