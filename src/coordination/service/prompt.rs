// @feature coordination
// @spec docs/features/coordination.md
// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use super::{
    lifecycle_failure, NexusService, ProjectBinding, PromptResponse, Result, ServiceResponse,
    UserPromptInput,
};
use crate::memory::MemoryReadScope;
use anyhow::Context;

impl NexusService {
    pub(super) fn user_prompt(
        &self,
        input: UserPromptInput,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        let result = (|| -> Result<PromptResponse> {
            let project = self.resolve_project(binding, input.context.project_root.as_deref())?;
            let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
            store.touch_session(
                &project.id,
                &input.context.session_id,
                &input.context.agent,
                project.worktree.as_deref(),
                &self.loaded.hash,
            )?;
            let hash = blake3::hash(input.prompt.as_bytes()).to_hex().to_string();
            let synopsis = self.prompt_synopsis(&input.prompt);
            let full = self
                .loaded
                .config
                .privacy
                .store_full_prompts
                .then_some(input.prompt.as_str());
            store.record_prompt(
                &project.id,
                &input.context.session_id,
                synopsis.as_deref(),
                &hash,
                full,
            )?;
            let memory_context = if self.loaded.config.memory.enabled {
                store.activate_memory_context(
                    &MemoryReadScope::layered(project.id.clone()),
                    &input.context.agent,
                    &input.context.session_id,
                    self.memory_context_limits(),
                )?
            } else {
                None
            };
            let remediation_notices = store
                .remediation_notices(&project.id, &input.context.agent, &input.context.session_id)
                .unwrap_or_default();
            Ok(PromptResponse {
                permitted: true,
                recorded: true,
                project_id: project.id,
                prompt_hash: hash,
                memory_context,
                remediation_notices,
            })
        })();
        match result {
            Ok(response) => ServiceResponse::Prompt(response),
            Err(error) => ServiceResponse::Hook(lifecycle_failure(error)),
        }
    }

    fn prompt_synopsis(&self, prompt: &str) -> Option<String> {
        let privacy = &self.loaded.config.privacy;
        privacy.store_prompt_synopsis.then(|| {
            let ceiling = crate::coordination::api::MAX_SESSION_SUMMARY_BYTES;
            crate::coordination::domain::truncate_utf8(
                synopsis(prompt, privacy.prompt_synopsis_max_chars.min(ceiling)),
                ceiling,
            )
        })
    }
}

pub(super) fn synopsis(prompt: &str, max_chars: usize) -> String {
    let characters = prompt.chars().take_while(|&character| character != '.');
    // Emit separators before words, using adjacent characters rather than an
    // accumulator. Skip the separator introduced by the initial space sentinel.
    std::iter::once(' ')
        .chain(characters.clone())
        .zip(characters)
        .filter(|(_, current)| !current.is_whitespace())
        .flat_map(|(previous, current)| {
            previous
                .is_whitespace()
                .then_some(' ')
                .into_iter()
                .chain(std::iter::once(current))
        })
        .skip(1)
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::synopsis;

    #[test]
    fn synopsis_normalizes_whitespace_before_applying_unicode_character_bounds() {
        for (prompt, normalized) in [
            ("", ""),
            (".ignored", ""),
            (" \t\n\r ", ""),
            (" \talpha\u{2003} beta  .ignored", "alpha beta"),
            ("é🙂\u{a0}界  e\u{301}. ignored", "é🙂 界 e\u{301}"),
            ("alpha! beta? gamma. ignored", "alpha! beta? gamma"),
            ("alpha\n\t", "alpha"),
        ] {
            for limit in 0..=normalized.chars().count() + 2 {
                assert_eq!(
                    synopsis(prompt, limit),
                    normalized.chars().take(limit).collect::<String>(),
                    "prompt={prompt:?}, limit={limit}"
                );
            }
        }
    }

    #[test]
    fn synopsis_stops_at_its_output_bound_before_scanning_a_large_tail() {
        let prompt = format!("  alpha   beta {}", "x ".repeat(500_000));

        assert_eq!(synopsis(&prompt, 10), "alpha beta");
        assert_eq!(
            synopsis(" first   sentence. ignored", 100),
            "first sentence"
        );
        assert_eq!(synopsis("ignored", 0), "");
    }
}
