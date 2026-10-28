// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary compose-literal-json: Compose config output doubles dollar characters; retained models store literal values and materialization escapes interpolation exactly once.
use anyhow::Result;
use serde_json::Value;
fn map_strings(value: &mut Value, transform: fn(&str) -> String) {
    match value {
        Value::String(text) => *text = transform(text),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| map_strings(value, transform)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| map_strings(value, transform)),
        _ => (),
    }
}
pub(super) fn decode(bytes: &[u8]) -> Result<Value> {
    let mut model = serde_json::from_slice(bytes)?;
    map_strings(&mut model, |value| value.replace("$$", "$"));
    Ok(model)
}
pub(super) fn encode(model: &Value) -> Result<Vec<u8>> {
    let mut escaped = model.clone();
    map_strings(&mut escaped, |value| value.replace('$', "$$"));
    Ok(serde_json::to_vec(&escaped)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_values_survive_repeated_compose_rendering() {
        let model =
            serde_json::json!({"environment":{"SECRET":"a$b$$c${D}"},"command":["echo $HOME"]});
        assert_eq!(decode(&encode(&model).unwrap()).unwrap(), model);
    }
}
