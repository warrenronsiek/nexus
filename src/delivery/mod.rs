// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
// @entrypoint DeliveryAction
mod environment;
mod origin;
mod records;
mod remediation;
pub use environment::*;
pub(crate) mod validation;
mod watch;
pub use origin::*;
pub use records::*;
pub use remediation::*;
pub use watch::*;

pub(crate) fn provider_pr_identity(provider: &str, repository: &str, number: u64) -> String {
    crate::identity::stable_id(&[
        "delivery-provider-pr",
        &provider.to_ascii_lowercase(),
        repository,
        &number.to_string(),
    ])
}
