// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
use crate::delivery::{DeliveryAction, DeliveryResponse};
use serde::Serialize;

pub type DeliveryCommand = super::ContextCommand<DeliveryAction>;

#[derive(Debug, Serialize)]
pub struct DeliveryEnvelope {
    pub ok: bool,
    #[serde(flatten)]
    pub response: DeliveryResponse,
}

#[derive(Debug, Serialize)]
pub struct DeliveryErrorEnvelope {
    pub ok: bool,
    pub error_code: &'static str,
    pub error: String,
}
