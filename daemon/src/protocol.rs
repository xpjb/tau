pub use tau_protocol::*;

pub trait ResponseError {
    fn command_failure(request_id: String, error: anyhow::Error) -> Self;
}
impl ResponseError for ServerMessage {
    fn command_failure(request_id: String, error: anyhow::Error) -> Self {
        let uncertain = error.is::<UncertainOutcome>();
        let mut response = Self::failure(request_id, error.to_string());
        if let Self::Response { uncertain: field, .. } = &mut response { *field = uncertain; }
        response
    }
}

/// Preserve uncertainty when a nested creation resumes an interrupted journal.
#[derive(Debug)]
pub(crate) struct UncertainOutcome(pub String);
impl std::fmt::Display for UncertainOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
}
impl std::error::Error for UncertainOutcome {}
