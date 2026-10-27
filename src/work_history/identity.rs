// @feature work-history
// @spec docs/features/work-history.md
pub(crate) fn host_work_identity(agent: &str, session: &str) -> String {
    crate::identity::stable_id(&["work", agent, session])
}
pub(crate) fn host_agent_identity(agent: &str, session: &str) -> String {
    crate::identity::stable_id(&["agent", agent, session])
}
