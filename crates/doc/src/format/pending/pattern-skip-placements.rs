//! Save rectangular exclusions by grid position and allow individual exclusions in a path
//! pattern. Earlier rectangular selections use the saved design's layout when first opened.
//! The last release warns about the newer format and preserves fields it cannot read.

use serde_json::Value;

pub fn step(value: &mut Value) -> crate::Result<()> {
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    let upgrades: Vec<_> = features
        .iter()
        .enumerate()
        .filter_map(|(index, feature)| {
            let saved = feature.get("pattern")?;
            (saved.get("type").and_then(Value::as_str) == Some("rectangular")
                && ["skip", "skip_members"].iter().any(|key| saved.get(*key).and_then(Value::as_array).is_some_and(|a| !a.is_empty())))
            .then(|| serde_json::from_value::<crate::PatternKind>(saved.clone()).ok().map(|kind| (index, kind)))
            .flatten()
        })
        .collect();
    if upgrades.is_empty() {
        return Ok(());
    }
    let mut parameters = crate::Document::new("");
    parameters.params = value.get("params").and_then(|p| serde_json::from_value(p.clone()).ok()).unwrap_or_default();
    if let Some(units) = value.get("units").and_then(Value::as_str) {
        parameters.units = units.into();
    }
    // Quantity can also refer to a named input on a feature, such as a sketch dimension.
    parameters.features = features
        .iter()
        .filter(|f| f.get("param_names").and_then(Value::as_array).is_some_and(|names| !names.is_empty()))
        .filter_map(|f| serde_json::from_value(f.clone()).ok())
        .collect();
    let (values, _) = parameters.param_values();
    let Some(features) = value.get_mut("features").and_then(Value::as_array_mut) else { return Ok(()) };
    for (index, mut kind) in upgrades {
        if kind.pin_rectangular_skips(&values).is_err() {
            continue;
        }
        let Some(saved) = features.get_mut(index).and_then(|f| f.get_mut("pattern")) else { continue };
        if let crate::PatternKind::Rectangular { skip_width, skip_members, .. } = kind {
            if let Some(width) = skip_width {
                saved["skip_width"] = Value::from(width);
            }
            if !skip_members.is_empty() {
                saved["skip_members"] = serde_json::to_value(skip_members).map_err(|err| crate::DocError::Invalid(err.to_string()))?;
            }
        }
    }
    Ok(())
}
