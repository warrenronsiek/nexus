// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
// @entrypoint NexusService::delivery_command
use super::NexusService;
use crate::coordination::{
    api::{DeliveryCommand, DeliveryEnvelope, DeliveryErrorEnvelope, ServiceResponse},
    workspace::ProjectBinding,
};
use crate::delivery::DeliveryRejection;
use crate::work_history::WorkActor;
use anyhow::{Context, Result};

use super::history_ops::history_service_adapter;

history_service_adapter!(
    command = DeliveryCommand, actor = WorkActor, rejection = DeliveryRejection,
    envelope = DeliveryEnvelope => Delivery,
    error = DeliveryErrorEnvelope => DeliveryError,
    method = delivery_command, apply = apply_delivery_command, label = "delivery coordination"
);
