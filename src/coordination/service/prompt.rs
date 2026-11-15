// @feature coordination
// @spec docs/features/coordination.md
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
