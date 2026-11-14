#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use solidcore_record::Vec2;
use solidcraft_sketch::Sketch;

use super::TOOL_BUILDS;
use crate::{Document, FeatureKind, Model, PlaneRef};

/// A sketch on XY of `lines`, each from its first point to its second.
fn lines_sketch(lines: &[[[f64; 2]; 2]]) -> FeatureKind {
    let mut sk = Sketch::new();
    for [a, b] in lines {
        let a = sk.add_point(Vec2::new(a[0], a[1]), None).unwrap();
        let b = sk.add_point(Vec2::new(b[0], b[1]), None).unwrap();
        sk.add_line_pts(a, b, None).unwrap();
    }
    FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }
}

/// A Frame Member on sketch `sketch`, in `size` square tube.
fn member(sketch: u64, size: f64, extra: serde_json::Value) -> FeatureKind {
    let size = format!("{size}");
    let mut p = serde_json::json!({"type": "frame_member", "path_sketch": sketch,
        "profile": {"source": "library", "profile": {"kind": "rect_tube", "width": size, "height": size, "wall": "2"}}});
    for (k, v) in extra.as_object().into_iter().flatten() {
        p[k] = v.clone();
    }
    serde_json::from_value(p).unwrap()
}

#[test]
fn each_earlier_tool_is_built_once() {
    // A: one 40 tube along X, y −20…20. B: three uprights ending on it at x = 100, 300, 500, fitted to it. Their
    // three ends meet the one tube, whose tool is built once.
    let mut doc = Document::new("earlier");
    let a = doc.add_feature(lines_sketch(&[[[0.0, 0.0], [600.0, 0.0]]]), None).unwrap();
    doc.add_feature(member(a, 40.0, serde_json::json!({"body_name": "A"})), None).unwrap();
    let b = doc
        .add_feature(lines_sketch(&[[[100.0, 0.0], [100.0, 300.0]], [[300.0, 0.0], [300.0, 300.0]], [[500.0, 0.0], [500.0, 300.0]]]), None)
        .unwrap();
    doc.add_feature(member(b, 25.0, serde_json::json!({"corner": "auto", "fit_ends": true, "body_name": "B"})), None).unwrap();
    let mut model = Model::new();
    TOOL_BUILDS.set(0);
    model.evaluate(&doc);
    assert!(model.results.iter().all(|r| r.error.is_none()), "{:?}", model.results.iter().find(|r| r.error.is_some()));
    assert_eq!(TOOL_BUILDS.get(), 1);
    // Each upright is fitted to the tube's near face, y = 20, its far end square at 300.
    let st = model.state();
    let uprights: Vec<_> = st.bodies.iter().filter(|b| b.name.starts_with('B')).collect();
    assert_eq!(uprights.len(), 3);
    for b in uprights {
        let bb = b.body.exact_bounds().unwrap();
        assert!((bb.min.y - 20.0).abs() < 1e-6 && (bb.max.y - 300.0).abs() < 1e-6, "{}: {bb:?}", b.name);
    }
}
