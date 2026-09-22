pub use tau_protocol::*;

pub trait ResponseError {
    fn command_failure(request_id: String, error: anyhow::Error) -> Self;
}
impl ResponseError for ServerMessage {
    fn command_failure(request_id: String, error: anyhow::Error) -> Self {
        let mut response = Self::failure(request_id, error.to_string());
        if let Self::Response { uncertain, .. } = &mut response {
            *uncertain = error.is::<crate::pi::UnconfirmedCommand>();
        }
        response
    }
}

pub trait ContextUsagePi {
    fn from_pi(data: &serde_json::Value) -> Option<Self>
    where
        Self: Sized;
}
impl ContextUsagePi for ContextUsage {
    fn from_pi(data: &serde_json::Value) -> Option<Self> {
        let usage: Self = serde_json::from_value(data.get("contextUsage")?.clone()).ok()?;
        (usage.context_window > 0
            && usage.context_window <= 9_007_199_254_740_991
            && usage
                .tokens
                .is_none_or(|tokens| tokens <= 9_007_199_254_740_991))
        .then_some(usage)
    }
}
