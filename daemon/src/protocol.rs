pub use tau_protocol::*;

pub trait ResponseError {
    fn command_failure(request_id: String, error: anyhow::Error) -> Self;
}
impl ResponseError for ServerMessage {
    fn command_failure(request_id: String, error: anyhow::Error) -> Self {
        Self::failure(request_id, error.to_string())
    }
}
