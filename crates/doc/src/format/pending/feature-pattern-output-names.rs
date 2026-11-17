//! Feature patterns store body names by output slot, including skipped copies. Name lists from
//! older patterns with exclusions are marked to read in the order the copies were made. Once
//! evaluated, the session puts those names in their slots without renaming existing bodies.
//! The last release warns about the newer format and preserves fields it cannot read.

use serde_json::Value;

pub fn step(doc: &mut Value) -> crate::Result<()> {
    let Some(Value::Array(features)) = doc.get_mut("features") else { return Ok(()) };
    for feature in features {
        let Value::Object(fields) = feature else { continue };
        let has_items = |key: &str| fields.get(key).and_then(Value::as_array).is_some_and(|items| !items.is_empty());
        if fields.get("type").and_then(Value::as_str) == Some("pattern")
            && has_items("features")
            && !has_items("bodies")
            && has_items("body_names")
            && fields.get("pattern").is_some_and(|pattern| {
                ["skip", "skip_members"].iter().any(|key| pattern.get(*key).and_then(Value::as_array).is_some_and(|items| !items.is_empty()))
            })
        {
            fields.insert("pattern_names_in_order".into(), Value::Bool(true));
        }
    }
    Ok(())
}
