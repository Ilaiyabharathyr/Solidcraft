//! Regression tests: freshly drawn rectangles take every ordinary edit without being reported
//! over-constrained (dragging, dimensioning, pinning the centre, making them square).

use serde_json::{Value, json};
use solidcraft_geom::prelude::*;

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// A new sketch with one rectangle of the given kind; returns its four side ids.
fn rect(kind: &str) -> (Session, Vec<String>) {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    let r = match kind {
        "center" => run(&mut s, "sketch.rectangle.center", json!({"center": [3, 2], "corner": [13, 9]})),
        "two" => run(&mut s, "sketch.rectangle.two_point", json!({"p0": [1, 1], "p1": [21, 15]})),
        _ => run(&mut s, "sketch.rectangle.three_point", json!({"p0": [0, 0], "p1": [20, 5], "p2": [18, 15], "connect": false})),
    };
    let sides = ids(&r["curves"]);
    assert_eq!(sides.len(), 4, "{r}");
    (s, sides)
}

fn healthy(s: &mut Session, what: &str) -> i64 {
    let si = run(s, "sketch.inspect", json!({}));
    assert_ne!(si["status"], "conflict", "{what}: {si}");
    assert!(si["status"].as_str().is_none_or(|t| !t.contains("over")), "{what}: {si}");
    si["dof"].as_i64().unwrap_or(-1)
}

const KINDS: [&str; 3] = ["center", "two", "three"];

#[test]
fn fresh_rectangles_have_the_expected_freedom() {
    for k in KINDS {
        let (mut s, _) = rect(k);
        let dof = healthy(&mut s, k);
        // Centre/two-point: x, y, width, height. Three-point adds the rotation.
        let want = if k == "three" { 5 } else { 4 };
        assert_eq!(dof, want, "{k}");
    }
}

#[test]
fn rectangles_take_dimensions_without_conflict() {
    for k in KINDS {
        let (mut s, l) = rect(k);
        run(&mut s, "sketch.dimension", json!({"entities": [l[0]], "value": 30}));
        healthy(&mut s, &format!("{k} width"));
        run(&mut s, "sketch.dimension", json!({"entities": [l[1]], "value": 12}));
        let dof = healthy(&mut s, &format!("{k} width+height"));
        assert_eq!(dof, if k == "three" { 3 } else { 2 }, "{k}");
    }
}

#[test]
fn rectangles_can_be_dragged_pinned_and_made_square() {
    for k in KINDS {
        // Drag a corner, then the middle of an edge.
        let (mut s, l) = rect(k);
        run(&mut s, "sketch.move_point", json!({"point": format!("{}.start", l[0]), "to": [-4, -3]}));
        healthy(&mut s, &format!("{k} drag corner"));
        run(&mut s, "sketch.move_point", json!({"point": format!("{}.end", l[2]), "to": [25, 22]}));
        healthy(&mut s, &format!("{k} drag other corner"));
        // Pin a corner to the origin.
        run(&mut s, "sketch.constraint.coincident", json!({"a": format!("{}.start", l[0]), "b": "origin"}));
        healthy(&mut s, &format!("{k} corner on origin"));
        // Square: equal sides.
        run(&mut s, "sketch.constraint.equal", json!({"a": l[0], "b": l[1]}));
        healthy(&mut s, &format!("{k} square"));
        run(&mut s, "sketch.dimension", json!({"entities": [l[0]], "value": 10}));
        let dof = healthy(&mut s, &format!("{k} square sized"));
        assert_eq!(dof, if k == "three" { 1 } else { 0 }, "{k}");
    }
}

#[test]
fn dragging_corners_never_collapses_a_side() {
    for k in KINDS {
        let (mut s, l) = rect(k);
        for (i, to) in [[-4.0, -3.0], [25.0, 22.0], [30.0, -5.0], [-8.0, 18.0]].iter().enumerate() {
            let pt = format!("{}.start", l[i % 4]);
            run(&mut s, "sketch.move_point", json!({"point": pt, "to": to}));
            let sk = s.model.state().sketch(s.active_sketch.unwrap()).unwrap().sketch.clone();
            for c in &l {
                let ci = sk.curve_index(c).unwrap();
                let pi = sk.curves[ci].kind.point_ids();
                let len = sk.point(pi[0]).unwrap().dist(sk.point(pi[1]).unwrap());
                assert!(len > 0.5, "{k}: side {c} collapsed to {len} after dragging {pt}");
            }
            healthy(&mut s, &format!("{k} drag {i}"));
        }
    }
}

#[test]
fn center_rectangle_draws_both_diagonals_like_fusion() {
    let (mut s, _) = rect("center");
    let si = run(&mut s, "sketch.inspect", json!({}));
    let curves = si["curves"].as_array().cloned().unwrap_or_default();
    // Four sides plus two construction diagonals; the other rectangles have no diagonals.
    assert_eq!(curves.len(), 6, "{si}");
    assert_eq!(curves.iter().filter(|c| c["construction"] == true).count(), 2, "{si}");
    assert_eq!(healthy(&mut s, "center diagonals"), 4);
    for k in ["two", "three"] {
        let (mut s, _) = rect(k);
        let si = run(&mut s, "sketch.inspect", json!({}));
        assert_eq!(si["curves"].as_array().map(Vec::len), Some(4), "{k}: {si}");
    }
}

/// A sketch on XY with two fixed lines: `base` along y = 0 (x 0 to 60) and `side` along x = 50
/// (y -30 to 40).
fn with_lines() -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [60, 0]], "ids": ["base"]}));
    run(&mut s, "sketch.line", json!({"points": [[50, -30], [50, 40]], "ids": ["side"]}));
    for line in ["base", "side"] {
        run(&mut s, "sketch.constraint.fix", json!({"entity": line}));
    }
    assert_eq!(healthy(&mut s, "the lines"), 0);
    s
}

/// The ids of the sketch's constraints of `kind`, as `sketch.inspect` names it.
fn constraints_of(s: &mut Session, kind: &str) -> Vec<String> {
    let si = run(s, "sketch.inspect", json!({}));
    si["constraints"].as_array().into_iter().flatten().filter(|c| c["type"] == kind).filter_map(|c| c["id"].as_str().map(str::to_string)).collect()
}

fn point_count(s: &Session) -> usize {
    s.model.state().sketch(s.active_sketch.unwrap()).unwrap().sketch.points.len()
}

/// #18756: a 2-Point Rectangle corner given as an on-curve snap is held on that curve by one
/// constraint, whichever way the rectangle was dragged, with `connect` off too; it is the
/// rectangle's own corner (no helper point), and deleting the constraint frees it again.
#[test]
fn a_two_point_corner_snapped_onto_a_curve_is_held_there_and_can_be_freed() {
    let named = ["bottom_left", "bottom_right", "top_right", "top_left"];
    for (i, p1) in [[40, 25], [5, 25], [5, -25], [40, -25]].into_iter().enumerate() {
        for connect in [true, false] {
            let mut s = with_lines();
            let before = point_count(&s);
            let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": "on:base:20,0", "p1": p1, "connect": connect}));
            assert_eq!(point_count(&s), before + 4, "{p1:?}: the snapped corner is the rectangle's own");
            let held = constraints_of(&mut s, "point_on_curve");
            assert_eq!(held.len(), 1, "{p1:?} connect {connect}: {r}");
            assert!(ids(&r["constraints"]).contains(&held[0]), "the reply names the constraint that holds the corner: {r}");
            assert!(constraints_of(&mut s, "coincident").len() <= 1, "{p1:?}: only the base's own tie to the origin");
            assert_eq!(healthy(&mut s, "held"), 3, "{p1:?} connect {connect}: x along the line, width, height");
            // The corner on the line is the one `p0` names, where it was given.
            let corner = r["corners"][named[i]].as_str().unwrap().to_string();
            let si = run(&mut s, "sketch.inspect", json!({}));
            let at = si["points"].as_array().unwrap().iter().find(|q| q["id"] == corner.as_str()).map(|q| q["at"].clone()).unwrap();
            assert!((at[0].as_f64().unwrap() - 20.0).abs() < 1e-9 && at[1].as_f64().unwrap().abs() < 1e-9, "{p1:?}: {corner} at {at}");
            run(&mut s, "sketch.delete", json!({"entities": held}));
            assert_eq!(healthy(&mut s, "freed"), 4, "{p1:?}: a free rectangle again");
        }
    }
}

/// Both corners snapped: each is held on its own curve. A midpoint and a crossing hold a corner
/// in both directions.
#[test]
fn two_point_corners_take_every_sketch_snap() {
    let mut s = with_lines();
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": "on:base:20,0", "p1": "on:side:50,25"}));
    assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 2);
    assert_eq!(healthy(&mut s, "both corners on curves"), 2);

    let mut s = with_lines();
    let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": "mid:base", "p1": [40, 25]}));
    assert_eq!(constraints_of(&mut s, "midpoint").len(), 1, "{r}");
    assert_eq!(healthy(&mut s, "a corner at a midpoint"), 2);

    let mut s = with_lines();
    let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": [20, 25], "p1": "x:base+side:50,0"}));
    let held = constraints_of(&mut s, "point_on_curve");
    assert_eq!(held.len(), 2, "one per curve of the crossing: {r}");
    assert!(held.iter().all(|k| ids(&r["constraints"]).contains(k)), "the reply names both: {r}");
    assert_eq!(healthy(&mut s, "a corner at a crossing"), 2);
    // The crossing corner is the bottom-right one.
    let si = run(&mut s, "sketch.inspect", json!({}));
    let corner = r["corners"]["bottom_right"].as_str().unwrap();
    let at = si["points"].as_array().unwrap().iter().find(|q| q["id"] == corner).map(|q| q["at"].clone()).unwrap();
    assert!((at[0].as_f64().unwrap() - 50.0).abs() < 1e-9 && at[1].as_f64().unwrap().abs() < 1e-9, "{at}");
    // A crossing is two constraints: each one deleted gives one freedom back.
    run(&mut s, "sketch.delete", json!({"entities": [held[0]]}));
    assert_eq!(healthy(&mut s, "one curve of the crossing let go"), 3);
    run(&mut s, "sketch.delete", json!({"entities": [held[1]]}));
    assert_eq!(healthy(&mut s, "freed from the crossing"), 4);
}

/// The Center Rectangle's centre and corner take the snaps too: the centre stays the point its
/// diagonal's midpoint constraint holds.
#[test]
fn a_center_rectangle_takes_snaps_for_its_centre_and_corner() {
    let mut s = with_lines();
    let before = point_count(&s);
    let r = run(&mut s, "sketch.rectangle.center", json!({"center": "on:base:20,0", "corner": "on:side:50,12"}));
    assert_eq!(point_count(&s), before + 5, "four corners and the centre: {r}");
    assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 2, "{r}");
    assert_eq!(constraints_of(&mut s, "midpoint").len(), 1, "{r}");
    assert_eq!(healthy(&mut s, "centre and corner on curves"), 2);
    let top_right = r["corners"]["top_right"].as_str().unwrap();
    let si = run(&mut s, "sketch.inspect", json!({}));
    let at = si["points"].as_array().unwrap().iter().find(|q| q["id"] == top_right).map(|q| q["at"].clone()).unwrap();
    assert!((at[0].as_f64().unwrap() - 50.0).abs() < 1e-9 && (at[1].as_f64().unwrap() - 12.0).abs() < 1e-9, "{at}");
    // Only the centre snapped, in each quadrant: one constraint, freed by deleting it.
    for corner in [[30, 8], [10, 8], [10, -8], [30, -8]] {
        let mut s = with_lines();
        let r = run(&mut s, "sketch.rectangle.center", json!({"center": "on:base:20,0", "corner": corner, "connect": false}));
        let held = constraints_of(&mut s, "point_on_curve");
        assert_eq!(held.len(), 1, "{corner:?}");
        // The snapped centre is the first point the command reports as created.
        let centre = ids(&r["created"]["points"]).first().cloned().unwrap();
        let si = run(&mut s, "sketch.inspect", json!({}));
        let at = si["points"].as_array().unwrap().iter().find(|q| q["id"] == centre.as_str()).map(|q| q["at"].clone()).unwrap();
        assert!((at[0].as_f64().unwrap() - 20.0).abs() < 1e-9 && at[1].as_f64().unwrap().abs() < 1e-9, "{corner:?}: {centre} at {at}");
        assert_eq!(healthy(&mut s, "centre held"), 3, "{corner:?}");
        run(&mut s, "sketch.delete", json!({"entities": held}));
        assert_eq!(healthy(&mut s, "centre freed"), 4, "{corner:?}");
    }
}

/// A corner snapped to a model vertex is a point of its own joined to the projected vertex by one
/// Coincident, with `connect` on or off, so that deleting the Coincident frees the rectangle; a
/// corner on a model edge is held on the projected edge.
#[test]
fn a_corner_snapped_to_the_model_is_tied_once_and_can_be_freed() {
    // The second vertex is at the sketch's origin: the corner is joined to the vertex alone, not
    // to the origin as well, so one deletion frees it there too.
    for (vertex, p1) in [([60, 0, 10], [40, 15]), ([0, 0, 10], [20, 15])] {
        for connect in [true, false] {
            let mut s = Session::default();
            run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 10}));
            run(&mut s, "sketch.create", json!({"plane": {"face": [30, 20, 10]}, "project_edges": false}));
            let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": {"vertex": vertex}, "p1": p1, "connect": connect}));
            let tied = constraints_of(&mut s, "coincident");
            assert_eq!(tied.len(), 1, "{vertex:?} connect {connect}: {r}");
            assert_eq!(healthy(&mut s, "a corner on a vertex"), 2, "{vertex:?} connect {connect}");
            run(&mut s, "sketch.delete", json!({"entities": tied}));
            assert_eq!(healthy(&mut s, "freed from the vertex"), 4, "{vertex:?} connect {connect}");
        }
    }
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 10}));
    run(&mut s, "sketch.create", json!({"plane": {"face": [30, 20, 10]}, "project_edges": false}));
    let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": {"on_edge": [20, 0, 10]}, "p1": [45, 25]}));
    assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 1, "{r}");
    assert_eq!(healthy(&mut s, "a corner on a model edge"), 3);
}

/// A snap that makes no point is refused, and a rectangle refused after its snap made a point
/// leaves the sketch as it was.
#[test]
fn a_bad_snap_or_a_refused_rectangle_leaves_the_sketch_alone() {
    let mut s = with_lines();
    let before = (point_count(&s), run(&mut s, "sketch.inspect", json!({}))["constraints"].clone());
    let calls = [
        ("sketch.rectangle.two_point", json!({"p0": "on:base:abc", "p1": [40, 25]}), "must be a point [x, y] or a snap"),
        ("sketch.rectangle.two_point", json!({"p0": "on:nope:20,0", "p1": [40, 25]}), "unknown curve"),
        ("sketch.rectangle.two_point", json!({"p0": "on:base:20,0", "p1": [20, 25]}), "must differ in x and y"),
        ("sketch.rectangle.two_point", json!({"p0": "on:base:20,3", "p1": [40, 0]}), "must differ in x and y"),
        ("sketch.rectangle.center", json!({"center": "mid:base", "corner": [30, 9]}), "must differ from the centre"),
        ("sketch.rectangle.center", json!({"center": [10, 10], "corner": "on:side:abc"}), "must be a point [x, y] or a snap"),
    ];
    for (id, p, why) in calls {
        let e = s.execute(id, &p).map(|r| r.to_string()).unwrap_err().to_string();
        assert!(e.contains(why), "{id} {p}: {e}");
        let after = (point_count(&s), run(&mut s, "sketch.inspect", json!({}))["constraints"].clone());
        assert_eq!(after, before, "{id} {p}: the refused call changed the sketch");
    }
}

/// Typed sizes and a held corner: with one corner on a curve both sizes go in as driving
/// dimensions and one freedom is left. With both corners on parallel lines the lines set the
/// height: a typed height that agrees is placed as a reference, one that disagrees is refused and
/// nothing is drawn, and a size as drawn is left out.
#[test]
fn typed_sizes_do_not_fight_a_corner_held_on_a_curve() {
    let request = |params: Value, dimensions: Value, as_drawn: bool| json!({"command": "sketch.rectangle.two_point", "params": params, "dimensions": dimensions, "as_drawn": as_drawn});
    let mut s = with_lines();
    run(&mut s, "sketch.draw.dimensioned", request(json!({"p0": "on:base:20,0", "p1": [40, 25]}), json!({"width": 30, "height": 12}), false));
    assert_eq!(healthy(&mut s, "one corner held, both sizes typed"), 1);

    let parallel = || {
        let mut s = with_lines();
        run(&mut s, "sketch.line", json!({"points": [[0, 25], [60, 25]], "ids": ["top"]}));
        run(&mut s, "sketch.constraint.fix", json!({"entity": "top"}));
        s
    };
    let between = json!({"p0": "on:base:20,0", "p1": "on:top:40,25"});

    let mut s = parallel();
    let r = run(&mut s, "sketch.draw.dimensioned", request(between.clone(), json!({"height": 25}), false));
    assert_eq!(healthy(&mut s, "a typed height the lines already set"), 2, "{r}");
    assert_eq!(r["dimensions"]["height"]["driven"], true, "the typed height is a reference: {r}");

    let mut s = parallel();
    let before = point_count(&s);
    let e =
        s.execute("sketch.draw.dimensioned", &request(between.clone(), json!({"height": 12}), false)).map(|r| r.to_string()).unwrap_err().to_string();
    assert!(e.contains("over-constrain"), "{e}");
    assert_eq!(point_count(&s), before, "the refused rectangle is not drawn");
    assert_eq!(healthy(&mut s, "after the refusal"), 0);

    // "Dimension new shapes" (sizes as drawn): one held corner takes both sizes.
    let mut s = with_lines();
    let r = run(&mut s, "sketch.draw.dimensioned", request(json!({"p0": "on:base:20,0", "p1": [40, 25]}), json!({}), true));
    assert!(r["dimensions"].get("width").is_some() && r["dimensions"].get("height").is_some(), "{r}");
    assert_eq!(healthy(&mut s, "one corner held, sizes as drawn"), 1, "{r}");

    let mut s = parallel();
    let r = run(&mut s, "sketch.draw.dimensioned", request(between, json!({}), true));
    assert_eq!(healthy(&mut s, "sizes as drawn"), 1, "{r}");
    assert!(r["dimensions"].get("height").is_none(), "the height is not dimensioned again: {r}");
    assert!(r["dimensions"].get("width").is_some(), "{r}");
}

/// A snapped corner with the other corner placed by parameters: the rectangle follows the
/// parameters and the snapped corner stays on its line, whichever diagonal was drawn.
#[test]
fn a_snapped_corner_agrees_with_a_corner_placed_by_parameters() {
    for (p1, name) in [(json!(["W", "H"]), "bottom_left"), (json!(["W", "-H"]), "top_left")] {
        let mut s = with_lines();
        run(&mut s, "parameters.add", json!({"name": "W", "expression": "40 mm"}));
        run(&mut s, "parameters.add", json!({"name": "H", "expression": "25 mm"}));
        let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": "on:base:10,0", "p1": p1}));
        let dof = healthy(&mut s, "drawn");
        run(&mut s, "parameters.change", json!({"name": "W", "expression": "45 mm"}));
        assert_eq!(healthy(&mut s, "after the parameter change"), dof, "{p1}");
        let corner = r["corners"][name].as_str().unwrap();
        let si = run(&mut s, "sketch.inspect", json!({}));
        let at = si["points"].as_array().unwrap().iter().find(|q| q["id"] == corner).map(|q| q["at"].clone()).unwrap();
        assert!(at[1].as_f64().unwrap().abs() < 1e-9, "{p1}: the snapped corner left the line: {at}, dof {dof}");
        assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 1, "{p1}");
    }
}

/// A snap is made in the sketch being edited: a snapped corner in a call that names another
/// sketch is refused and changes neither sketch. Coordinates still draw in the named sketch, and
/// naming the sketch being edited takes the snap.
#[test]
fn a_snapped_corner_is_refused_for_a_sketch_that_is_not_being_edited() {
    let mut s = Session::default();
    let other = run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Other"}))["sketch"].as_u64().unwrap();
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [60, 0]], "ids": ["base"]}));
    run(&mut s, "sketch.finish", json!({}));
    let edited = run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "Edited"}))["sketch"].as_u64().unwrap();
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [60, 0]], "ids": ["base"]}));
    let points = |s: &Session, id: u64| s.model.state().sketch(id).unwrap().sketch.points.len();
    let before = (points(&s, other), points(&s, edited));
    for (id, p) in [
        ("sketch.rectangle.two_point", json!({"sketch": other, "p0": "on:base:20,0", "p1": [40, 25]})),
        ("sketch.rectangle.two_point", json!({"sketch": "Other", "p0": [20, 5], "p1": "mid:base"})),
        ("sketch.rectangle.center", json!({"sketch": other, "center": "on:base:20,0", "corner": [30, 8]})),
        ("sketch.rectangle.center", json!({"sketch": other, "center": [20, 5], "corner": "on:base:30,0"})),
    ] {
        let e = s.execute(id, &p).map(|r| r.to_string()).unwrap_err().to_string();
        assert!(e.contains("only in the sketch being edited"), "{id} {p}: {e}");
        assert_eq!((points(&s, other), points(&s, edited)), before, "{id} {p}: a refused call changed a sketch");
    }
    run(&mut s, "sketch.rectangle.two_point", json!({"sketch": other, "p0": [20, 5], "p1": [40, 25]}));
    assert_eq!((points(&s, other), points(&s, edited)), (before.0 + 4, before.1));
    run(&mut s, "sketch.rectangle.two_point", json!({"sketch": edited, "p0": "on:base:20,0", "p1": [40, 25]}));
    assert_eq!(points(&s, edited), before.1 + 4);
    assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 1);
}

/// The Center Rectangle's centre takes a model vertex, a midpoint and a crossing: one tie to the
/// vertex that frees it when deleted, and the midpoint and the crossing hold it in both directions.
#[test]
fn a_center_rectangle_centre_takes_a_vertex_a_midpoint_and_a_crossing() {
    for connect in [true, false] {
        let mut s = Session::default();
        run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 10}));
        run(&mut s, "sketch.create", json!({"plane": {"face": [30, 20, 10]}, "project_edges": false}));
        let r = run(&mut s, "sketch.rectangle.center", json!({"center": {"vertex": [60, 0, 10]}, "corner": [50, 8], "connect": connect}));
        let tied = constraints_of(&mut s, "coincident");
        assert_eq!(tied.len(), 1, "connect {connect}: {r}");
        assert_eq!(healthy(&mut s, "the centre on a vertex"), 2, "connect {connect}");
        run(&mut s, "sketch.delete", json!({"entities": tied}));
        assert_eq!(healthy(&mut s, "the centre freed"), 4, "connect {connect}");
    }
    let mut s = with_lines();
    let r = run(&mut s, "sketch.rectangle.center", json!({"center": "mid:base", "corner": [40, 8]}));
    assert_eq!(constraints_of(&mut s, "midpoint").len(), 2, "the diagonal's and the snap's: {r}");
    assert_eq!(healthy(&mut s, "the centre at a midpoint"), 2);
    let mut s = with_lines();
    let r = run(&mut s, "sketch.rectangle.center", json!({"center": "x:base+side:50,0", "corner": [58, 8]}));
    assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 2, "{r}");
    assert_eq!(healthy(&mut s, "the centre at a crossing"), 2);
}

/// "Also place them from the origin" (`locate`) with a corner held on a line: the held corner is
/// among the points it places, and the rectangle ends fully constrained.
#[test]
fn locate_places_a_rectangle_with_a_held_corner() {
    let mut s = with_lines();
    let r = run(
        &mut s,
        "sketch.draw.dimensioned",
        json!({"command": "sketch.rectangle.two_point", "params": {"p0": "on:base:20,0", "p1": [40, 25]}, "dimensions": {}, "as_drawn": true, "locate": true}),
    );
    assert_eq!(healthy(&mut s, "located"), 0, "{r}");
    assert_eq!(r["located"].as_array().map(Vec::len), Some(1), "one distance from the origin, along the line: {r}");
    assert_eq!(constraints_of(&mut s, "point_on_curve").len(), 1, "{r}");
    // The held corner is the snap's point, made before the command's own: it must be among the
    // points the command reports as created, which are the ones placing from the origin goes
    // through, and it is the first of them.
    let held = r["corners"]["bottom_left"].as_str().unwrap();
    assert_eq!(ids(&r["created"]["points"]).first().map(String::as_str), Some(held), "{r}");
}

/// In a drawing sheet's sketch the rectangles draw from coordinates as before, and a snapped
/// corner is refused in the words every sheet snap is, with nothing drawn.
#[test]
fn a_sheet_sketch_draws_rectangles_from_coordinates_and_refuses_a_snap() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 5}));
    let drawing = run(&mut s, "drawing.create", json!({"name": "Sheet"}));
    run(&mut s, "drawing.sketch.create", json!({"owner": drawing["sheets"][0]["id"]}));
    let base = run(&mut s, "sketch.line", json!({"points": [[10, 10], [70, 10]]}));
    let base = ids(&base["curves"])[0].clone();
    let r = run(&mut s, "sketch.rectangle.two_point", json!({"p0": [15, 20], "p1": [55, 40]}));
    assert_eq!(ids(&r["curves"]).len(), 4, "{r}");
    let r = run(&mut s, "sketch.rectangle.center", json!({"center": [100, 30], "corner": [110, 36]}));
    assert_eq!(ids(&r["curves"]).len(), 4, "{r}");
    for (id, p) in [
        ("sketch.rectangle.two_point", json!({"p0": format!("on:{base}:20,10"), "p1": [40, 50]})),
        ("sketch.rectangle.two_point", json!({"p0": [20, 50], "p1": format!("mid:{base}")})),
        ("sketch.rectangle.center", json!({"center": format!("on:{base}:20,10"), "corner": [30, 16]})),
    ] {
        let e = s.execute(id, &p).map(|r| r.to_string()).unwrap_err().to_string();
        assert!(e.contains("not supported yet: automatic snap or projection in a sheet sketch"), "{id} {p}: {e}");
    }
}
