// @feature coordination
// @spec docs/features/coordination.md
pub(super) fn synopsis(prompt: &str, max_chars: usize) -> String {
    let mut result = String::with_capacity(max_chars.min(prompt.len()));
    let mut chars = 0_usize;
    let mut pending_space = false;
    for character in prompt.chars() {
        if character == '.' {
            break;
        }
        if character.is_whitespace() {
            pending_space |= !result.is_empty();
            continue;
        }
        if pending_space {
            result.push(' ');
            chars += 1;
            if chars == max_chars {
                break;
            }
            pending_space = false;
        }
        if chars == max_chars {
            break;
        }
        result.push(character);
        chars += 1;
        if chars == max_chars {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::synopsis;

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
