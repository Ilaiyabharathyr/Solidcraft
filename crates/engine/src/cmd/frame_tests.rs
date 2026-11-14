#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use solidcraft_geom::prelude::*;
use std::f64::consts::PI;

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-12)
}

fn new_sketch() -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    s
}

/// The areas of the active sketch's profiles, largest first.
fn profile_areas(s: &Session) -> Vec<f64> {
    let id = s.active_sketch.unwrap();
    let mut a: Vec<f64> = s.model.state().sketch(id).unwrap().profiles.iter().map(|p| p.area).collect();
    a.sort_by(|x, y| y.total_cmp(x));
    a
}

#[test]
fn list_gives_every_kind_with_a_working_example() {
    let mut s = Session::default();
    let out = run(&mut s, "frame.profile.list", json!({}));
    let kinds = out["kinds"].as_array().unwrap();
    let ids: Vec<&str> = kinds.iter().filter_map(|k| k["kind"].as_str()).collect();
    assert_eq!(ids, ["bar", "round_bar", "rect_tube", "round_tube", "angle", "channel", "i_beam"]);
    for k in kinds {
        let r = run(&mut s, "frame.profile.properties", json!({"profile": k["example"]}));
        assert!(f(&r["area"]) > 0.0, "{}", k["kind"]);
    }
}

#[test]
fn properties_fill_defaults_and_give_mass_per_metre() {
    let mut s = Session::default();
    let r = run(
        &mut s,
        "frame.profile.properties",
        json!({"profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3, "outer_radius": 0}, "density": 7850}),
    );
    let area = 40.0 * 40.0 - 34.0 * 34.0;
    assert!(rel(f(&r["area"]), area) < 1e-12, "{r}");
    assert!(rel(f(&r["mass_per_m"]), area * 1e-6 * 7850.0) < 1e-12, "{r}");
    assert!(rel(f(&r["ixx"]), (40f64.powi(4) - 34f64.powi(4)) / 12.0) < 1e-12, "{r}");
    let r = run(&mut s, "frame.profile.properties", json!({"profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}}));
    assert_eq!(r["profile"]["outer_radius"], json!(6.0), "{r}");
    assert!(r.get("mass_per_m").is_none(), "{r}");
}

#[test]
fn bad_profiles_are_refused_with_the_reason() {
    let mut s = new_sketch();
    for (p, why) in [
        (json!({}), "`profile` is required"),
        (json!({"profile": {"kind": "hexagon"}}), "`profile`"),
        (json!({"profile": {"kind": "rect_tube", "width": 10, "height": 10, "wall": 5}}), "wall"),
        (json!({"profile": {"kind": "round_bar", "diameter": -1}}), "diameter"),
        (json!({"profile": {"kind": "round_bar", "diameter": 10}, "density": -1}), "density"),
        (json!({"profile": {"kind": "round_bar", "diameter": 10}, "density": "steel"}), "density"),
    ] {
        let err = s.execute("frame.profile.properties", &p).unwrap_err().to_string();
        assert!(err.contains(why), "{p}: {err}");
    }
    for p in [
        json!({"profile": {"kind": "round_bar", "diameter": 10}, "at": [1e300, 0]}),
        json!({"profile": {"kind": "round_bar", "diameter": 10}, "at": "here"}),
        json!({"profile": {"kind": "round_bar", "diameter": 10}, "anchor": "middle"}),
        json!({"profile": {"kind": "round_bar", "diameter": 10}, "angle": "a lot"}),
        json!({"profile": {"kind": "round_bar", "diameter": 10}, "fixed": 3}),
    ] {
        assert!(s.execute("frame.profile.insert", &p).is_err(), "{p} is refused");
    }
    // Nothing was drawn.
    assert!(profile_areas(&s).is_empty());
}

#[test]
fn insert_needs_a_sketch() {
    let mut s = Session::default();
    assert!(s.execute("frame.profile.insert", &json!({"profile": {"kind": "round_bar", "diameter": 10}})).is_err());
}

#[test]
fn inserted_tube_is_one_profile_with_its_hole() {
    let mut s = new_sketch();
    let r = run(&mut s, "frame.profile.insert", json!({"profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}}));
    let area = f(&r["section"]["area"]);
    // Outer radius 6, inner radius 3.
    let want = (1600.0 - (4.0 - PI) * 36.0) - (34.0 * 34.0 - (4.0 - PI) * 9.0);
    assert!(rel(area, want) < 1e-12, "{r}");
    // Four lines and four arcs outside, the same inside.
    assert_eq!(r["created"]["curves"].as_array().unwrap().len(), 16, "{r}");
    // The sketch has the wall and, as a region of its own, the inside.
    let areas = profile_areas(&s);
    assert_eq!(areas.len(), 2, "{areas:?}");
    assert!(areas.iter().any(|a| rel(*a, want) < 1e-9), "the wall is the tube's section: {areas:?}");
}

#[test]
fn inserted_channel_extrudes_to_its_area_times_length() {
    let mut s = new_sketch();
    let r = run(
        &mut s,
        "frame.profile.insert",
        json!({"profile": {"kind": "channel", "height": 60, "width": 45, "web": 5, "flange": 6}, "at": [100, 50], "anchor": "bottom_left"}),
    );
    let area = f(&r["section"]["area"]);
    let areas = profile_areas(&s);
    assert!(areas.iter().any(|a| rel(*a, area) < 1e-9), "{areas:?} vs {area}");
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance": 500, "profiles": [{"point": [101.5, 51.5]}]}));
    let m = run(&mut s, "inspect.measure", json!({}));
    let v = f(&m["total"]["volume_mm3"]);
    // The kernel measures arcs through their facets: within 1e-4.
    assert!(rel(v, area * 500.0) < 1e-4, "volume {v} vs {}", area * 500.0);
    // The bottom-left corner sits on (100, 50): the body spans 100…145 and 50…110.
    let bb = &m["bodies"][0]["bbox"];
    let near = |v: &Value, want: f64| (f(v) - want).abs() < 1e-6;
    assert!(near(&bb["min"][0], 100.0) && near(&bb["min"][1], 50.0) && near(&bb["max"][0], 145.0) && near(&bb["max"][1], 110.0), "{bb}");
}

#[test]
fn inserted_profile_turns_about_its_anchor_and_stays_fixed() {
    let mut s = new_sketch();
    let r = run(
        &mut s,
        "frame.profile.insert",
        json!({"profile": {"kind": "angle", "width": 60, "height": 40, "thickness": 5}, "anchor": "bottom_left", "angle": PI / 2.0}),
    );
    let id = s.active_sketch.unwrap();
    let sk = s.model.state().sketch(id).unwrap().sketch.clone();
    let created: Vec<&str> = r["created"]["curves"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(!created.is_empty());
    for c in sk.curves.iter().filter(|c| created.contains(&c.id.as_str())) {
        assert!(c.fixed, "{} is locked", c.id);
    }
    // Turned a quarter about the heel at the origin: the 60 mm leg runs up the y axis, the 40 mm
    // one along −x.
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &sk.points {
        lo = [lo[0].min(p.pos.x), lo[1].min(p.pos.y)];
        hi = [hi[0].max(p.pos.x), hi[1].max(p.pos.y)];
    }
    assert!((lo[0] + 40.0).abs() < 1e-9 && hi[0].abs() < 1e-9 && lo[1].abs() < 1e-9 && (hi[1] - 60.0).abs() < 1e-9, "{lo:?} {hi:?}");
    // Unlocked on request.
    let r =
        run(&mut s, "frame.profile.insert", json!({"profile": {"kind": "round_tube", "diameter": 20, "wall": 2}, "at": [200, 0], "fixed": false}));
    assert_eq!(r["created"]["curves"].as_array().unwrap().len(), 2, "two circles: {r}");
    let sk = s.model.state().sketch(id).unwrap().sketch.clone();
    assert!(sk.curves.iter().filter(|c| r["created"]["curves"].as_array().unwrap().contains(&json!(c.id))).all(|c| !c.fixed));
}

#[test]
fn insert_is_one_undo_step() {
    let mut s = new_sketch();
    run(&mut s, "frame.profile.insert", json!({"profile": {"kind": "i_beam", "height": 100, "width": 50, "web": 4.5, "flange": 7}}));
    assert_eq!(profile_areas(&s).len(), 1);
    run(&mut s, "edit.undo", json!({}));
    assert!(profile_areas(&s).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Members

/// How close a measured volume comes: the kernel measures curved faces through their facets.
const MEASURED: f64 = 1e-3;

/// The section area of a 40 × 40 × 3 rectangular tube (outer radius 6, inner 3).
fn tube_area() -> f64 {
    (1600.0 - (4.0 - PI) * 36.0) - (34.0 * 34.0 - (4.0 - PI) * 9.0)
}

fn tube() -> Value {
    json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3})
}

/// A body's name, volume, bounding box and centre of mass.
type Measured = (String, f64, [f64; 3], [f64; 3], [f64; 3]);

fn bodies(s: &mut Session) -> Vec<Measured> {
    let m = run(s, "inspect.measure", json!({}));
    let v3 = |v: &Value| [f(&v[0]), f(&v[1]), f(&v[2])];
    m["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (b["name"].as_str().unwrap().to_string(), f(&b["volume_mm3"]), v3(&b["bbox"]["min"]), v3(&b["bbox"]["max"]), v3(&b["center_of_mass"]))
        })
        .collect()
}

fn total_volume(s: &mut Session) -> f64 {
    bodies(s).iter().map(|b| b.1).sum()
}

fn near3(got: [f64; 3], want: [f64; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-6)
}

/// A 600 × 400 rectangle on XY, as the frame sketch `Frame`.
fn rectangle_frame(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(s, "sketch.finish", json!({}));
}

#[test]
fn members_follow_every_line_of_a_rectangle() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let r = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 4, "{r}");
    let names: Vec<&str> = bs.iter().map(|b| b.0.as_str()).collect();
    assert_eq!(names, ["Rect Tube 40x40x3", "Rect Tube 40x40x3 (2)", "Rect Tube 40x40x3 (3)", "Rect Tube 40x40x3 (4)"]);
    // Each member is the section run the length of its line: 2 × 600 + 2 × 400.
    let v = total_volume(&mut s);
    assert!(rel(v, tube_area() * 2000.0) < MEASURED, "{v} vs {}", tube_area() * 2000.0);
    // The bottom line runs along +X: the profile is centred on it, square to it.
    assert!(bs.iter().any(|b| near3(b.2, [0.0, -20.0, -20.0]) && near3(b.3, [600.0, 20.0, 20.0])), "{bs:?}");
    // The right line runs along Y at x = 600.
    assert!(bs.iter().any(|b| near3(b.2, [580.0, 0.0, -20.0]) && near3(b.3, [620.0, 400.0, 20.0])), "{bs:?}");
}

/// A member's body name counts characters, as the refusal says: a hundred `ł` (200 bytes) is
/// taken, 129 characters are not.
#[test]
fn member_body_names_count_characters_not_bytes() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let long = "ł".repeat(100);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "body_name": long}));
    assert_eq!(bodies(&mut s)[0].0, long);
    let error = s.execute("frame.member", &json!({"sketch": "Frame", "profile": tube(), "body_name": "ł".repeat(129)})).unwrap_err().to_string();
    assert!(error.contains("1…128 characters"), "{error}");
    // So does each name in `names`.
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "names": {"l1": long}}));
    let error = s.execute("frame.member", &json!({"sketch": "Frame", "profile": tube(), "names": {"l1": "ł".repeat(129)}})).unwrap_err().to_string();
    assert!(error.contains("1…128 characters"), "{error}");
}

#[test]
fn members_are_made_of_a_standard_size_by_name() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": {"size": "RSC 203x89"}}));
    let bs = bodies(&mut s);
    let names: Vec<&str> = bs.iter().map(|b| b.0.as_str()).collect();
    assert_eq!(names, ["RSC 203x89", "RSC 203x89 (2)", "RSC 203x89 (3)", "RSC 203x89 (4)"]);
    // Each member is the size's section, 37.9 cm² in the BS 4 table, the length of its line.
    let props = run(&mut s, "frame.profile.properties", json!({"profile": {"size": "RSC 203x89"}}));
    let area = f(&props["area"]);
    assert!(rel(area, 3790.0) < 0.005, "{props}");
    assert_eq!(props["size"], json!("RSC 203x89"));
    assert_eq!(props["profile"]["flange_slope"], json!(5.0), "{props}");
    let v = total_volume(&mut s);
    assert!(rel(v, area * 2000.0) < MEASURED, "{v} vs {}", area * 2000.0);
    // The bottom line runs along +X: the channel, 88.9 wide and 203.2 deep, is centred on it.
    assert!(bs.iter().any(|b| near3(b.2, [0.0, -44.45, -101.6]) && near3(b.3, [600.0, 44.45, 101.6])), "{bs:?}");
    // The design keeps the size's name and dimensions, and reopens to the same members.
    let saved = s.doc.to_json();
    assert!(saved.contains("RSC 203x89") && saved.contains("5 deg"), "{saved}");
    let mut reopened = Session::new(solidcraft_doc::Document::from_json(&saved).unwrap());
    let after = bodies(&mut reopened);
    assert_eq!(after.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), names);
    assert!(rel(total_volume(&mut reopened), v) < 1e-9);
    // The reported profile, slope in degrees, is the same section given by its dimensions.
    let again = run(&mut s, "frame.profile.properties", json!({"profile": props["profile"]}));
    assert!(rel(f(&again["area"]), area) < 1e-12 && rel(f(&again["iyy"]), f(&props["iyy"])) < 1e-12, "{again}");
    // In an inch design the size is still in millimetres.
    run(&mut s, "document.units", json!({"units": "in"}));
    let inch = run(&mut s, "frame.profile.properties", json!({"profile": {"size": "SHS 100x100x5"}}));
    assert!((f(&inch["width"]) - 100.0).abs() < 1e-9, "{inch}");
}

#[test]
fn standard_sizes_are_listed_and_refused_with_the_reason() {
    let mut s = Session::default();
    let out = run(&mut s, "frame.profile.list", json!({}));
    let sizes = out["sizes"].as_array().unwrap();
    for family in sizes {
        let names = family["names"].as_array().unwrap();
        assert!(!names.is_empty() && !family["source"].as_str().unwrap_or_default().is_empty(), "{family}");
        // The first and last size of each family make a section of their kind.
        for name in [&names[0], &names[names.len() - 1]] {
            let r = run(&mut s, "frame.profile.properties", json!({"profile": {"size": name}}));
            assert_eq!(r["profile"]["kind"], family["kind"], "{r}");
        }
    }
    let rsc = sizes.iter().find(|f| f["family"] == "RSC").unwrap();
    for name in ["RSC 203x89", "RSC 152x76"] {
        assert!(rsc["names"].as_array().unwrap().contains(&json!(name)), "{rsc}");
    }
    let only = run(&mut s, "frame.profile.list", json!({"family": "SHS"}));
    assert_eq!(only["sizes"].as_array().map(Vec::len), Some(1));
    assert!(s.execute("frame.profile.list", &json!({"family": "XYZ"})).unwrap_err().to_string().contains("`family`"));
    rectangle_frame(&mut s);
    for (profile, why) in [
        (json!({"size": "RSC 203x89", "height": 200}), "brings its own dimensions"),
        (json!({"size": "RSC 999x99"}), "unknown size"),
        (json!({"size": "RSC 203x89", "kind": "rect_tube"}), "is a channel profile"),
    ] {
        let err = s.execute("frame.member", &json!({"sketch": "Frame", "profile": profile})).unwrap_err().to_string();
        assert!(err.contains(why), "{profile}: {err}");
    }
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn members_follow_a_parameter_edit() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "W", "expression": "600 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], ["W", 0], ["W", 400], [0, 400]], "closed": true}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}));
    assert!(rel(total_volume(&mut s), tube_area() * 2000.0) < MEASURED);
    run(&mut s, "parameters.change", json!({"name": "W", "expression": "800 mm"}));
    // The two 600 mm members are 800 mm now.
    let v = total_volume(&mut s);
    assert!(rel(v, tube_area() * 2400.0) < MEASURED, "{v} vs {}", tube_area() * 2400.0);
}

#[test]
fn members_along_3d_sketch_lines() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Tower"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 1500], [600, 0, 1500]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Tower", "profile": tube()}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    // The upright: square to Z, the profile's x along world X.
    assert!(bs.iter().any(|b| near3(b.2, [-20.0, -20.0, 0.0]) && near3(b.3, [20.0, 20.0, 1500.0])), "{bs:?}");
    // The rail along X at the top.
    assert!(bs.iter().any(|b| near3(b.2, [0.0, -20.0, 1480.0]) && near3(b.3, [600.0, 20.0, 1520.0])), "{bs:?}");
    let v = total_volume(&mut s);
    assert!(rel(v, tube_area() * 2100.0) < MEASURED, "{v} vs {} {bs:?}", tube_area() * 2100.0);
}

#[test]
fn a_profile_sketch_is_the_section() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Tube section"}));
    run(&mut s, "sketch.circle.center", json!({"center": [0, 0], "radius": 20}));
    run(&mut s, "sketch.circle.center", json!({"center": [0, 0], "radius": 17}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "Post"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [1000, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    // The material: the ring between the circles (nested loops alternate).
    run(&mut s, "frame.member", json!({"sketch": "Post", "profile_sketch": "Tube section"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].0, "Tube section");
    let ring = PI * (400.0 - 289.0) * 1000.0;
    assert!(rel(bs[0].1, ring) < MEASURED, "{} vs {ring}", bs[0].1);
    // Picked: the inner disc alone.
    run(&mut s, "frame.member", json!({"sketch": "Post", "profile_sketch": "Tube section", "profiles": [{"point": [0, 0]}]}));
    // A second feature of the same profile numbers on from the first.
    let disc = bodies(&mut s).into_iter().find(|b| b.0 != "Tube section").unwrap();
    assert_eq!(disc.0, "Tube section (2)");
    assert!(rel(disc.1, PI * 289.0 * 1000.0) < MEASURED, "{disc:?}");
}

#[test]
fn anchor_angle_and_mirror_place_the_profile() {
    let place = |extra: Value| {
        let mut s = Session::default();
        run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rail"}));
        run(&mut s, "sketch.line", json!({"points": [[0, 0], [1000, 0]]}));
        run(&mut s, "sketch.finish", json!({}));
        let mut p = json!({"sketch": "Rail", "profile": {"kind": "angle", "width": 60, "height": 40, "thickness": 5}, "anchor": "bottom_left"});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        run(&mut s, "frame.member", p);
        bodies(&mut s).remove(0)
    };
    // Along +X the profile's x is world +Y and its y world +Z: the heel on the line, the legs
    // running +Y (60) and +Z (40).
    let plain = place(json!({}));
    assert!(near3(plain.2, [0.0, 0.0, 0.0]) && near3(plain.3, [1000.0, 60.0, 40.0]), "{plain:?}");
    // Turned a quarter: the 60 mm leg up, the 40 mm one toward −Y.
    let b = place(json!({"angle": "90 deg"}));
    assert!(near3(b.2, [0.0, -40.0, 0.0]) && near3(b.3, [1000.0, 0.0, 60.0]), "{b:?}");
    // Mirrored: the same box, the heel at the far side (the mass sits past the middle).
    let m = place(json!({"mirror": true}));
    assert!(near3(m.2, [0.0, 0.0, 0.0]) && near3(m.3, [1000.0, 60.0, 40.0]), "{m:?}");
    assert!(plain.4[1] < 30.0 && m.4[1] > 30.0, "{plain:?} {m:?}");
}

#[test]
fn a_located_member_keeps_its_profile_point_on_the_line_when_the_section_changes() {
    let place = |width: f64, height: f64, locate: Value| {
        let mut s = Session::default();
        run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rail"}));
        run(&mut s, "sketch.line", json!({"points": [[0, 0], [1000, 0]]}));
        run(&mut s, "sketch.finish", json!({}));
        let p = json!({"sketch": "Rail", "profile": {"kind": "angle", "width": width, "height": height, "thickness": 5}, "locate": locate});
        run(&mut s, "frame.member", p);
        bodies(&mut s).remove(0)
    };
    // The angle is drawn about its heel: locate [0, 0] puts the heel on the line, as the
    // bottom_left anchor does, and it stays there when the legs grow.
    let small = place(60.0, 40.0, json!([0, 0]));
    assert!(near3(small.2, [0.0, 0.0, 0.0]) && near3(small.3, [1000.0, 60.0, 40.0]), "{small:?}");
    let large = place(80.0, 50.0, json!([0, 0]));
    assert!(near3(large.2, [0.0, 0.0, 0.0]) && near3(large.3, [1000.0, 80.0, 50.0]), "{large:?}");
    // A point 10 across and 5 up from the heel on the line moves the profile back by that much.
    let off = place(60.0, 40.0, json!([10, 5]));
    assert!(near3(off.2, [0.0, -10.0, -5.0]) && near3(off.3, [1000.0, 50.0, 35.0]), "{off:?}");
    // A bad point is refused with the reason.
    let mut s = Session::default();
    rectangle_frame(&mut s);
    for bad in [json!([1]), json!(["a", 0]), json!([0, 1e9])] {
        let err = s.execute("frame.member", &json!({"sketch": "Frame", "profile": tube(), "locate": bad})).unwrap_err().to_string();
        assert!(err.contains("`locate`"), "{bad}: {err}");
    }
    // A design file with a point the command would refuse fails the feature, with the reason.
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "locate": [0, 0]}));
    let mut saved: Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    fn far(v: &mut Value) {
        match v {
            Value::Object(o) => {
                if let Some(l) = o.get_mut("locate") {
                    *l = json!([1e8, 0.0]);
                }
                o.values_mut().for_each(far);
            }
            Value::Array(a) => a.iter_mut().for_each(far),
            _ => {}
        }
    }
    far(&mut saved);
    let reopened = Session::new(solidcraft_doc::Document::from_json(&saved.to_string()).unwrap());
    let fid = reopened.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().id;
    let failing = reopened.model.result(fid).and_then(|r| r.error.clone()).unwrap_or_default();
    assert!(failing.contains("`locate`"), "{failing}");
}

#[test]
fn bad_members_are_refused() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Arc"}));
    let arc = run(&mut s, "sketch.arc.three_point", json!({"start": [0, 0], "end": [100, 0], "through": [50, 30]}));
    let arc_id = arc["created"]["curves"][0].as_str().unwrap().to_string();
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Empty"}));
    run(&mut s, "sketch.finish", json!({}));
    for (p, why) in [
        (json!({"sketch": "Frame"}), "`profile`"),
        (json!({"sketch": "Frame", "profile": tube(), "profile_sketch": "Frame"}), "one of them"),
        (json!({"sketch": "Frame", "profile_sketch": "Frame"}), "must not be the frame sketch"),
        (json!({"sketch": "Nowhere", "profile": tube()}), "no sketch"),
        (json!({"sketch": "Frame", "profile": {"kind": "rect_tube", "width": 10, "height": 10, "wall": 5}}), "wall"),
        (json!({"sketch": "Frame", "profile": tube(), "lines": []}), "`lines`"),
        (json!({"sketch": "Frame", "profile": tube(), "lines": ["nope"]}), "nope"),
        (json!({"sketch": "Arc", "profile": tube(), "lines": [arc_id]}), "not a straight line"),
        (json!({"sketch": "Arc", "profile": tube()}), "no lines"),
        (json!({"sketch": "Empty", "profile": tube()}), "no lines"),
        (json!({"sketch": "Frame", "profile": tube(), "anchor": "middle"}), "`anchor`"),
        (json!({"sketch": "Frame", "profile": tube(), "angle": "a lot"}), "angle"),
        (json!({"sketch": "Frame", "profile": tube(), "mirror": 3}), "`mirror`"),
        (json!({"sketch": "Frame", "profile": tube(), "names": ["Rail"]}), "`names`"),
        (json!({"sketch": "Frame", "profile": tube(), "names": {"l1": " "}}), "`names`"),
    ] {
        let err = s.execute("frame.member", &p).unwrap_err().to_string();
        assert!(err.contains(why), "{p}: {err}");
    }
    assert!(bodies(&mut s).is_empty(), "nothing was made");
    // Not while a sketch is open.
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    assert!(s.execute("frame.member", &json!({"sketch": "Frame", "profile": tube()})).is_err());
}

#[test]
fn members_undo_and_survive_save_and_reopen() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "anchor": "bottom", "angle": "15 deg", "mirror": true, "body_name": "Rail"}),
    );
    let before = bodies(&mut s);
    assert_eq!(before.len(), 4);
    assert_eq!(before[0].0, "Rail");
    let mut reopened = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    let after = bodies(&mut reopened);
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(b.0, a.0);
        assert!(rel(a.1, b.1) < 1e-9 && near3(a.2, b.2) && near3(a.3, b.3), "{b:?} vs {a:?}");
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(bodies(&mut s).is_empty());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(bodies(&mut s).len(), 4);
}

/// The first two lines of the rectangle frame, by id.
fn two_lines(s: &Session) -> (String, String) {
    let l = sketch_lines(s, "Frame");
    (l[0].0.clone(), l[1].0.clone())
}

#[test]
fn corner_settings_are_taken_and_saved() {
    use solidcraft_doc::frame::{CornerOverride, CornerType};
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (l1, l2) = two_lines(&s);
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "corner": "butt", "flip": true,
               "corners": [{"members": [l1, l2], "corner": "mitre", "flip": false}]}),
    );
    let reopened = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    let kind = &reopened.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().kind;
    let solidcraft_doc::FeatureKind::FrameMember { corner, flip, corners, .. } = kind else { panic!("not a frame member") };
    assert_eq!((*corner, *flip), (CornerType::Butt, true));
    assert_eq!(*corners, [CornerOverride { members: vec![l1, l2], corner: CornerType::Mitre, flip: false }]);
}

#[test]
fn bad_corner_settings_are_refused() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (l1, l2) = two_lines(&s);
    let ok = json!({"members": [l1, l2], "corner": "mitre"});
    let many: Vec<Value> = (0..=solidcraft_doc::frame::MAX_MEMBERS).map(|_| ok.clone()).collect();
    let cases = [
        ("corner", json!({"corner": "round"})),
        ("flip", json!({"flip": "yes"})),
        ("corners", json!({"corners": "mitre"})),
        ("corners", json!({"corners": [{"members": [l1], "corner": "mitre"}]})),
        ("corners", json!({"corners": [{"members": [l1, "nope"], "corner": "mitre"}]})),
        ("corners", json!({"corners": [{"members": [l1, l1], "corner": "mitre"}]})),
        ("corners", json!({"corners": [ok, {"members": [l2, l1], "corner": "butt"}]})),
        ("corners", json!({"corners": many})),
    ];
    for (key, extra) in cases {
        let mut p = json!({"sketch": "Frame", "profile": tube()});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let e = s.execute("frame.member", &p).unwrap_err().to_string();
        assert!(e.contains(&format!("`{key}`")), "{extra:.80}: {e}");
    }
    assert!(s.doc.features.iter().all(|f| !matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })));
}

#[test]
fn a_design_without_corner_settings_builds_as_before() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 4);
    assert!(bs.iter().any(|b| near3(b.2, [0.0, -20.0, -20.0]) && near3(b.3, [600.0, 20.0, 20.0])), "{bs:?}");
    let json = s.doc.to_json();
    for key in ["\"corner\"", "\"flip\"", "\"corners\""] {
        assert!(!json.contains(key), "{key} saved");
    }
}

/// A frame sketch `Y` of three lines from the origin, by id.
fn three_at_a_point(s: &mut Session) -> [String; 3] {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Y"}));
    for end in [[600, 0], [0, 400], [-300, -300]] {
        run(s, "sketch.line", json!({"points": [[0, 0], end]}));
    }
    run(s, "sketch.finish", json!({}));
    let l = sketch_lines(s, "Y");
    [l[0].0.clone(), l[1].0.clone(), l[2].0.clone()]
}

#[test]
fn auto_and_fit_ends_are_taken_and_saved() {
    use solidcraft_doc::frame::{CornerOverride, CornerType};
    let mut s = Session::default();
    let ids = three_at_a_point(&mut s);
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Y", "profile": tube(), "corner": "auto", "fit_ends": true,
               "corners": [{"members": ids, "corner": "auto", "flip": true}]}),
    );
    let reopened = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    let kind = &reopened.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().kind;
    let solidcraft_doc::FeatureKind::FrameMember { corner, fit_ends, corners, .. } = kind else { panic!("not a frame member") };
    assert_eq!((*corner, *fit_ends), (CornerType::Auto, true));
    assert_eq!(*corners, [CornerOverride { members: ids.to_vec(), corner: CornerType::Auto, flip: true }]);
}

#[test]
fn an_old_two_member_override_still_loads() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (l1, l2) = two_lines(&s);
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "corner": "butt", "corners": [{"members": [l1, l2], "corner": "butt"}]}),
    );
    // The design's override as a stage 1 build wrote it, by hand: two ids and no `flip`.
    let old: Value = serde_json::from_str(&format!(r#"[{{"members":["{l1}","{l2}"],"corner":"mitre"}}]"#)).unwrap();
    fn put(v: &mut Value, old: &Value) -> bool {
        match v {
            Value::Object(o) if o.contains_key("corners") => {
                o.insert("corners".into(), old.clone());
                true
            }
            Value::Object(o) => o.values_mut().any(|v| put(v, old)),
            Value::Array(a) => a.iter_mut().any(|v| put(v, old)),
            _ => false,
        }
    }
    let mut doc: Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    assert!(put(&mut doc, &old), "no corners in {doc}");
    let mut reopened = Session::new(solidcraft_doc::Document::from_json(&doc.to_string()).unwrap());
    let kind = &reopened.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().kind;
    let solidcraft_doc::FeatureKind::FrameMember { corners, .. } = kind else { panic!("not a frame member") };
    assert_eq!((corners[0].members.clone(), corners[0].corner, corners[0].flip), (vec![l1, l2], solidcraft_doc::frame::CornerType::Mitre, false));
    assert_eq!(bodies(&mut reopened).len(), 4);
}

#[test]
fn bad_auto_settings_are_refused() {
    let mut s = Session::default();
    let ids = three_at_a_point(&mut s);
    let many: Vec<String> = (0..=solidcraft_doc::frame::MAX_MEMBERS).map(|k| format!("l{k}")).collect();
    let cases = [
        ("fit_ends", json!({"fit_ends": "yes"})),
        ("corners", json!({"corners": [{"members": [ids[0]], "corner": "auto"}]})),
        ("corners", json!({"corners": [{"members": [ids[0], ids[1], ids[0]], "corner": "auto"}]})),
        ("corners", json!({"corners": [{"members": many, "corner": "auto"}]})),
        ("corner", json!({"corner": "automatic"})),
    ];
    for (key, extra) in cases {
        let mut p = json!({"sketch": "Y", "profile": tube()});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let e = s.execute("frame.member", &p).unwrap_err().to_string();
        assert!(e.contains(&format!("`{key}`")), "{extra:.80}: {e}");
    }
    assert!(s.doc.features.iter().all(|f| !matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })));
}

#[test]
fn a_design_without_fit_ends_saves_without_it() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "fit_ends": false}));
    assert!(!s.doc.to_json().contains("fit_ends"));
}

#[test]
fn a_three_tube_override_on_a_mitre_frame_is_refused() {
    let mut s = Session::default();
    let ids = three_at_a_point(&mut s);
    let p = json!({"sketch": "Y", "profile": tube(), "corner": "mitre", "corners": [{"members": ids, "corner": "butt"}]});
    let e = s.execute("frame.member", &p).unwrap_err().to_string();
    assert!(e.contains("`corners`"), "{e}");
    run(&mut s, "frame.member", json!({"sketch": "Y", "profile": tube(), "corner": "auto", "corners": [{"members": ids, "corner": "butt"}]}));
}

#[test]
fn a_hand_edited_three_tube_override_on_a_mitre_frame_fails() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let l = sketch_lines(&s, "Frame");
    let (a, b, c) = (&l[0].0, &l[1].0, &l[2].0);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre", "corners": [{"members": [a, b], "corner": "butt"}]}));
    let member = s.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().id;
    let json = serde_json::from_str::<Value>(&s.doc.to_json()).unwrap().to_string();
    let from = format!("\"members\":[\"{a}\",\"{b}\"]");
    assert!(json.contains(&from), "{json}");
    let edited = json.replace(&from, &format!("\"members\":[\"{a}\",\"{b}\",\"{c}\"]"));
    let reopened = Session::new(solidcraft_doc::Document::from_json(&edited).unwrap());
    let e = reopened.model.result(member).and_then(|r| r.error.clone()).unwrap_or_default();
    assert!(e.contains("A corner override is not at a corner of the frame"), "{e}");
}

/// A frame sketch `V` of the lines through `points` (one line per pair of neighbours), with tube
/// members finished `corner`.
fn polyline_frame(points: Value, corner: &str) -> (Session, Result<Value, String>) {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "V"}));
    run(&mut s, "sketch.line", json!({"points": points}));
    run(&mut s, "sketch.finish", json!({}));
    let r = s.execute("frame.member", &json!({"sketch": "V", "profile": tube(), "corner": corner})).map_err(|e| e.to_string());
    (s, r)
}

#[test]
fn a_mitred_rectangle_meets_at_its_corners() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 4, "{bs:?}");
    // The bottom member runs out to the outer corners, and keeps its centreline's volume.
    let bottom = bs.iter().find(|b| near3(b.2, [-20.0, -20.0, -20.0]) && near3(b.3, [620.0, 20.0, 20.0])).unwrap_or_else(|| panic!("{bs:?}"));
    assert!(rel(bottom.1, tube_area() * 600.0) < MEASURED, "{bottom:?} vs {}", tube_area() * 600.0);
    assert!(bs.iter().any(|b| near3(b.2, [-20.0, -20.0, -20.0]) && near3(b.3, [20.0, 420.0, 20.0])), "{bs:?}");
    assert!(rel(total_volume(&mut s), tube_area() * 2000.0) < MEASURED);
}

#[test]
fn mitres_at_30_70_110_and_140_degrees() {
    for deg in [30.0f64, 70.0, 110.0, 140.0] {
        let (c, sn) = (deg.to_radians().cos(), deg.to_radians().sin());
        let (mut s, r) = polyline_frame(json!([[300, 0], [0, 0], [300.0 * c, 300.0 * sn]]), "mitre");
        r.unwrap_or_else(|e| panic!("{deg}: {e}"));
        let bs = bodies(&mut s);
        assert_eq!(bs.len(), 2, "{deg}: {bs:?}");
        for b in &bs {
            assert!(rel(b.1, tube_area() * 300.0) < MEASURED, "{deg}: {b:?} vs {}", tube_area() * 300.0);
        }
        // The member along X runs past the corner on its outer side, to where the mitre meets y = -20.
        let reach = -20.0 / (deg.to_radians() / 2.0).tan();
        assert!(bs.iter().any(|b| (b.2[0] - reach).abs() < 1e-6), "{deg}: {reach} {bs:?}");
    }
}

#[test]
fn a_corner_sharper_than_five_degrees_fails() {
    let a = 4.0f64.to_radians();
    let (mut s, r) = polyline_frame(json!([[300, 0], [0, 0], [300.0 * a.cos(), 300.0 * a.sin()]]), "mitre");
    let e = r.unwrap_err();
    assert!(e.contains("Corner at (0, 0, 0) is too sharp to finish"), "{e}");
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn members_in_line_are_left_square() {
    let (mut s, r) = polyline_frame(json!([[0, 0], [300, 0], [600, 0]]), "mitre");
    r.unwrap();
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    assert!(bs.iter().any(|b| near3(b.2, [0.0, -20.0, -20.0]) && near3(b.3, [300.0, 20.0, 20.0])), "{bs:?}");
    assert!(bs.iter().any(|b| near3(b.2, [300.0, -20.0, -20.0]) && near3(b.3, [600.0, 20.0, 20.0])), "{bs:?}");
}

#[test]
fn three_members_at_a_point_are_left_alone() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "T"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [300, 0], [600, 0]]}));
    run(&mut s, "sketch.line", json!({"points": [[300, 0], [300, 300]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "T", "profile": tube(), "corner": "mitre"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 3, "{bs:?}");
    for (lo, hi) in
        [([0.0, -20.0, -20.0], [300.0, 20.0, 20.0]), ([300.0, -20.0, -20.0], [600.0, 20.0, 20.0]), ([280.0, 0.0, -20.0], [320.0, 300.0, 20.0])]
    {
        assert!(bs.iter().any(|b| near3(b.2, lo) && near3(b.3, hi)), "{lo:?} {hi:?}: {bs:?}");
    }
}

#[test]
fn a_3d_sketch_corner_is_mitred() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Tower"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 600], [400, 300, 900]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Tower", "profile": tube(), "corner": "mitre"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    // The upright now reaches above its line's top, where the mitre's high side is.
    assert!(bs.iter().any(|b| b.3[2] > 600.0 + 1.0 && b.3[2] < 700.0 && rel(b.1, tube_area() * 600.0) < MEASURED), "{bs:?}");
    let slant = 340_000f64.sqrt();
    assert!(bs.iter().any(|b| rel(b.1, tube_area() * slant) < MEASURED), "{bs:?} vs {}", tube_area() * slant);
}

#[test]
fn mitred_members_keep_their_names() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre"}));
    let names: Vec<String> = bodies(&mut s).into_iter().map(|b| b.0).collect();
    assert_eq!(names, ["Rect Tube 40x40x3", "Rect Tube 40x40x3 (2)", "Rect Tube 40x40x3 (3)", "Rect Tube 40x40x3 (4)"]);
}

/// The rectangle frame's bottom (600) and left (400) members, by their boxes.
fn bottom_and_left(s: &mut Session) -> (Measured, Measured) {
    let bs = bodies(s);
    let bottom = bs.iter().find(|b| b.3[0] > 500.0 && b.3[1] < 200.0).unwrap_or_else(|| panic!("{bs:?}")).clone();
    let left = bs.iter().find(|b| b.3[0] < 200.0 && b.3[1] > 300.0).unwrap_or_else(|| panic!("{bs:?}")).clone();
    (bottom, left)
}

#[test]
fn a_butted_rectangle_runs_its_long_sides_through() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "butt"}));
    let (bottom, left) = bottom_and_left(&mut s);
    // The 600 members run out to the 400s' outer faces; the 400s stop at their near faces.
    assert!((bottom.2[0] + 20.0).abs() < 1e-6 && (bottom.3[0] - 620.0).abs() < 1e-6, "{bottom:?}");
    assert!(rel(bottom.1, tube_area() * 640.0) < MEASURED, "{bottom:?} vs {}", tube_area() * 640.0);
    assert!((left.2[1] - 20.0).abs() < 1e-6 && (left.3[1] - 380.0).abs() < 1e-6, "{left:?}");
    assert!(rel(left.1, tube_area() * 360.0) < MEASURED, "{left:?} vs {}", tube_area() * 360.0);
    // Each cut face is named for the end it finishes.
    let st = s.model.state();
    let names = solidcraft_doc::naming::face_names(st.body(&bottom.0).unwrap());
    for role in ["corner_start", "corner_end"] {
        assert!(names.iter().any(|n| n.contains(role)), "{role} in {names:?}");
    }
}

#[test]
fn flip_runs_the_short_sides_through() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "butt", "flip": true}));
    let (bottom, left) = bottom_and_left(&mut s);
    assert!((left.2[1] + 20.0).abs() < 1e-6 && (left.3[1] - 420.0).abs() < 1e-6, "{left:?}");
    assert!((bottom.2[0] - 20.0).abs() < 1e-6 && (bottom.3[0] - 580.0).abs() < 1e-6, "{bottom:?}");
}

#[test]
fn which_member_runs_through_follows_drawing_order_not_pick_order() {
    // A diamond: every side equally long and at 45° to X, so only drawing order can choose.
    let build = |reverse: bool| {
        let mut s = Session::default();
        run(&mut s, "sketch.create", json!({"plane": "XY", "name": "D"}));
        run(&mut s, "sketch.line", json!({"points": [[0, -200], [200, 0], [0, 200], [-200, 0], [0, -200]]}));
        run(&mut s, "sketch.finish", json!({}));
        let mut lines: Vec<String> = sketch_lines(&s, "D").into_iter().map(|l| l.0).collect();
        assert_eq!(lines.len(), 4);
        if reverse {
            lines.reverse();
        }
        let names: serde_json::Map<String, Value> = lines.iter().map(|l| (l.clone(), json!(l))).collect();
        run(&mut s, "frame.member", json!({"sketch": "D", "profile": tube(), "corner": "butt", "lines": lines, "names": names}));
        let faces: std::collections::BTreeMap<String, Vec<String>> = s
            .model
            .state()
            .bodies
            .iter()
            .map(|b| {
                let mut n = solidcraft_doc::naming::face_names(b).to_vec();
                n.sort();
                (b.name.clone(), n)
            })
            .collect();
        let measured: std::collections::BTreeMap<String, (f64, [f64; 3], [f64; 3])> =
            bodies(&mut s).into_iter().map(|b| (b.0, (b.1, b.2, b.3))).collect();
        (measured, faces)
    };
    let ((drawn, drawn_faces), (picked, picked_faces)) = (build(false), build(true));
    assert_eq!(drawn.keys().collect::<Vec<_>>(), picked.keys().collect::<Vec<_>>());
    for (line, (v, lo, hi)) in &drawn {
        let (pv, plo, phi) = picked[line];
        assert!(rel(pv, *v) < 1e-9 && near3(plo, *lo) && near3(phi, *hi), "{line}: drawn {drawn:?} picked {picked:?}");
    }
    assert_eq!(drawn_faces, picked_faces);
}

#[test]
fn a_coped_box_tube_keeps_slivers_in_the_through_member_radii() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let r = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "coped_butt"}));
    let (bottom, left) = bottom_and_left(&mut s);
    // T as in Butt; U's slivers fill T's outer radius (6), down to 20 − 6.
    assert!((bottom.2[0] + 20.0).abs() < 1e-6 && (bottom.3[0] - 620.0).abs() < 1e-6, "{r} {bottom:?}");
    assert!((left.2[1] - 14.0).abs() < 1e-3, "{left:?}");
    assert!((left.3[1] - 386.0).abs() < 1e-3, "{left:?}");
    assert_eq!(bodies(&mut s).len(), 4);
}

#[test]
fn a_coped_round_tube_is_fishmouthed() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let profile = json!({"kind": "round_tube", "diameter": 40, "wall": 3});
    let r = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": profile, "corner": "coped_butt"}));
    let (bottom, left) = bottom_and_left(&mut s);
    assert!((bottom.2[0] + 20.0).abs() < 1e-6 && (bottom.3[0] - 620.0).abs() < 1e-6, "{r} {bottom:?}");
    // U's tips reach T's centreline at its sides.
    assert!(left.2[1].abs() < 1e-3 && (left.3[1] - 400.0).abs() < 1e-3, "{left:?}");
    // A·360 + 2∫(20 − √(400 − z²)) dA over the annulus: each tip the depth T's outside leaves.
    assert!(rel(left.1, 129_458.17) < MEASURED, "{left:?} vs 129458.17");
}

/// The line of sketch `name` that rises off y = 0.
fn line_off_x_axis(s: &Session, name: &str) -> String {
    sketch_lines(s, name).into_iter().find(|(_, a, b)| a[1].max(b[1]) > 1.0).unwrap().0
}

#[test]
fn a_coped_member_too_short_to_reach_is_refused() {
    // 15 mm of U ends inside T: what is left of it lies on T's far side only.
    let (mut s, r) = polyline_frame(json!([[400, 0], [0, 0], [0, 15]]), "coped_butt");
    let u = line_off_x_axis(&s, "V");
    let e = r.unwrap_err();
    assert!(e.contains(&format!("the member on line `{u}` was cut away completely")), "{e}");
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn a_short_coped_brace_keeps_its_own_side() {
    let deg = 30.0f64.to_radians();
    let (mut s, r) = polyline_frame(json!([[400, 0], [0, 0], [70.0 * deg.cos(), 70.0 * deg.sin()]]), "coped_butt");
    r.unwrap_or_else(|e| panic!("{e}"));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    // U is the one that does not run along T; it stays above T's near face, less T's radius.
    let u = bs.iter().find(|b| b.3[0] - b.2[0] < 300.0).unwrap_or_else(|| panic!("{bs:?}"));
    assert!(u.2[1] > 14.0 - 1e-3, "{u:?}");
}

#[test]
fn a_short_coped_round_tube_keeps_its_fishmouth() {
    // 25 mm of round U comes 5 mm out of T; its tips reach T's centreline.
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "V"}));
    run(&mut s, "sketch.line", json!({"points": [[400, 0], [0, 0], [0, 25]]}));
    run(&mut s, "sketch.finish", json!({}));
    let profile = json!({"kind": "round_tube", "diameter": 40, "wall": 3});
    run(&mut s, "frame.member", json!({"sketch": "V", "profile": profile, "corner": "coped_butt"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    let u = bs.iter().find(|b| b.3[0] - b.2[0] < 300.0).unwrap_or_else(|| panic!("{bs:?}"));
    assert!(u.2[1].abs() < 1e-3 && (u.3[1] - 25.0).abs() < 1e-3, "{u:?}");
}

#[test]
fn butts_at_70_and_110_degrees_meet_flush() {
    for deg in [70.0f64, 110.0] {
        let (c, sn) = (deg.to_radians().cos(), deg.to_radians().sin());
        let (mut s, r) = polyline_frame(json!([[400, 0], [0, 0], [300.0 * c, 300.0 * sn]]), "butt");
        r.unwrap_or_else(|e| panic!("{deg}: {e}"));
        let bs = bodies(&mut s);
        assert_eq!(bs.len(), 2, "{deg}: {bs:?}");
        // T (the 400) runs h / sin φ past the corner to U's outer face; U stops that far short.
        let (t, u) = (tube_area() * (400.0 + 20.0 / sn), tube_area() * (300.0 - 20.0 / sn));
        assert!(bs.iter().any(|b| rel(b.1, t) < MEASURED), "{deg}: T {t} in {bs:?}");
        assert!(bs.iter().any(|b| rel(b.1, u) < MEASURED), "{deg}: U {u} in {bs:?}");
    }
}

#[test]
fn a_butt_at_150_degrees_is_flush() {
    let deg = 150.0f64.to_radians();
    let (mut s, r) = polyline_frame(json!([[400, 0], [0, 0], [300.0 * deg.cos(), 300.0 * deg.sin()]]), "butt");
    r.unwrap();
    let bs = bodies(&mut s);
    let t = bs.iter().find(|b| b.3[0] > 300.0).unwrap_or_else(|| panic!("{bs:?}"));
    let want = tube_area() * (400.0 + 20.0 / deg.sin());
    assert!(rel(t.1, want) < MEASURED, "{t:?} vs {want}");
    // T is cut on U's outer face: the plane 20 from the corner along m = (−sin 30°, −cos 30°),
    // facing −m. T's face y = +20 meets it furthest back: 0.5 (x + 10) + 0.866 (20 + 17.32) = 0,
    // so x = −20 (1 + cos 30°) / sin 150° = −74.641.
    let min_x = -20.0 * (1.0 + 30f64.to_radians().cos()) / deg.sin();
    assert!((t.2[0] - min_x).abs() < 1e-6, "{t:?} vs {min_x}");
}

#[test]
fn a_nearly_straight_butt_is_refused() {
    // At 175° T would run back 458 mm, past the far end of the 300 mm U.
    let deg = 175.0f64.to_radians();
    let (mut s, r) = polyline_frame(json!([[400, 0], [0, 0], [300.0 * deg.cos(), 300.0 * deg.sin()]]), "butt");
    let e = r.unwrap_err();
    assert!(e.contains("Corner at (0, 0, 0) is too flat to butt"), "{e}");
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn an_override_changes_only_its_corner() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (bottom_line, left_line) = (line_at_y(&s, "Frame", 0.0), line_at_x(&s, "Frame", 0.0));
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "corner": "mitre",
               "corners": [{"members": [bottom_line, left_line], "corner": "butt", "flip": true}]}),
    );
    let (bottom, left) = bottom_and_left(&mut s);
    // Flipped, the shorter left member runs through at the bottom; the bottom's far end is mitred.
    assert!((left.2[1] + 20.0).abs() < 1e-6, "{left:?}");
    assert!((bottom.2[0] - 20.0).abs() < 1e-6 && (bottom.3[0] - 620.0).abs() < 1e-6, "{bottom:?}");
}

#[test]
fn an_override_follows_its_corner_through_a_dimension_edit() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "W", "expression": "600 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], ["W", 0], ["W", 400], [0, 400]], "closed": true}));
    run(&mut s, "sketch.finish", json!({}));
    let (bottom_line, left_line) = (line_at_y(&s, "Frame", 0.0), line_at_x(&s, "Frame", 0.0));
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "corner": "mitre",
               "corners": [{"members": [left_line, bottom_line], "corner": "butt", "flip": true}]}),
    );
    run(&mut s, "parameters.change", json!({"name": "W", "expression": "800 mm"}));
    let bs = bodies(&mut s);
    let bottom = bs.iter().find(|b| b.3[0] > 700.0 && b.3[1] < 200.0).unwrap_or_else(|| panic!("{bs:?}"));
    let left = bs.iter().find(|b| b.3[0] < 200.0 && b.3[1] > 300.0).unwrap_or_else(|| panic!("{bs:?}"));
    assert!((left.2[1] + 20.0).abs() < 1e-6, "{left:?}");
    assert!((bottom.2[0] - 20.0).abs() < 1e-6 && (bottom.3[0] - 820.0).abs() < 1e-6, "{bottom:?}");
}

#[test]
fn an_override_whose_corner_has_gone_fails() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "V"}));
    run(&mut s, "sketch.line", json!({"points": [[400, 0], [0, 0]], "ids": ["a"]}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [0, 300]], "ids": ["b"]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "V", "profile": tube(), "corners": [{"members": ["a", "b"], "corner": "butt"}]}));
    assert_eq!(bodies(&mut s).len(), 2);
    // Line b drawn again 50 mm up: a and b no longer meet.
    run(&mut s, "sketch.edit", json!({"sketch": "V"}));
    run(&mut s, "sketch.delete", json!({"entities": ["b"]}));
    run(&mut s, "sketch.line", json!({"points": [[0, 50], [0, 300]], "ids": ["b"]}));
    let finished = s.execute("sketch.finish", &json!({})).map_err(|e| e.to_string());
    let member = s.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().id;
    let e = finished.err().or_else(|| s.model.result(member).and_then(|r| r.error.clone())).unwrap_or_default();
    assert!(e.contains("A corner override is not at a corner of the frame"), "{e} {:?}", sketch_lines(&s, "V"));
}

#[test]
fn a_member_too_short_for_its_cuts_fails_by_name() {
    let (mut s, r) = polyline_frame(json!([[0, 300], [0, 0], [30, 0], [30, 300]]), "butt");
    let short = line_at_y(&s, "V", 0.0);
    let e = r.unwrap_err();
    assert!(e.contains(&format!("the member on line `{short}` was cut away completely")), "{e}");
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn a_member_cut_into_pieces_fails_by_name() {
    // A 10 mm member mitred at both ends keeps only the section within 5 mm of its inner side:
    // a channel whose web is on its outer side keeps its two flanges, apart.
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "V"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 300], [0, 0], [10, 0], [10, 300]]}));
    run(&mut s, "sketch.finish", json!({}));
    let short = line_at_y(&s, "V", 0.0);
    let channel = json!({"kind": "channel", "height": 60, "width": 45, "web": 5, "flange": 6});
    let e = s.execute("frame.member", &json!({"sketch": "V", "profile": channel, "mirror": true, "corner": "mitre"})).unwrap_err().to_string();
    assert!(e.contains(&format!("the member on line `{short}` is cut into pieces by its corners")), "{e}");
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn a_bottom_left_anchor_butts_flush_with_the_placed_section() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "butt", "anchor": "bottom_left"}));
    let bs = bodies(&mut s);
    // The members sit off their lines: find them by their runs along X and Y.
    let t = bs.iter().find(|b| b.3[0] - b.2[0] > 500.0 && b.2[1] < 100.0).unwrap_or_else(|| panic!("{bs:?}"));
    let u = bs.iter().find(|b| b.3[1] - b.2[1] > 300.0 && b.2[0] < 100.0).unwrap_or_else(|| panic!("{bs:?}"));
    // T ends flush with U's outer face, and U ends on T's near face.
    assert!((t.2[0] - u.2[0]).abs() < 1e-6, "{t:?} {u:?}");
    assert!((u.2[1] - t.3[1]).abs() < 1e-6, "{t:?} {u:?}");
}

#[test]
fn a_hand_edited_override_list_is_refused_not_a_panic() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (bottom_line, left_line) = (line_at_y(&s, "Frame", 0.0), line_at_x(&s, "Frame", 0.0));
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "corner": "mitre",
               "corners": [{"members": [bottom_line, left_line], "corner": "butt"}]}),
    );
    let member = s.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().id;
    /// The saved override list: the array under `corners` beside `corner`.
    fn overrides(v: &mut Value) -> Option<&mut Vec<Value>> {
        if v.get("corner").is_some() && v.get("corners").is_some_and(Value::is_array) {
            return v.get_mut("corners")?.as_array_mut();
        }
        match v {
            Value::Object(m) => m.values_mut().find_map(overrides),
            Value::Array(a) => a.iter_mut().find_map(overrides),
            _ => None,
        }
    }
    let saved: Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    let one = overrides(&mut saved.clone()).unwrap()[0].clone();
    for (edit, why) in [
        (vec![json!({"members": [bottom_line, "nope"], "corner": "butt"})], "A corner override is not at a corner of the frame"),
        (vec![json!({"members": [bottom_line, bottom_line], "corner": "butt"})], "A corner override is not at a corner of the frame"),
        (vec![one; solidcraft_doc::frame::MAX_MEMBERS + 1], "corner overrides"),
    ] {
        let mut file = saved.clone();
        *overrides(&mut file).unwrap() = edit;
        let reopened = Session::new(solidcraft_doc::Document::from_json(&file.to_string()).unwrap());
        let e = reopened.model.result(member).and_then(|r| r.error.clone()).unwrap_or_default();
        assert!(e.contains(why), "{why}: {e}");
    }
}

// ---------------------------------------------------------------------------------------------
// Regressions from the adversarial review of #4003

/// The lines of sketch `name`, as (id, start, end) in sketch coordinates.
fn sketch_lines(s: &Session, name: &str) -> Vec<(String, [f64; 2], [f64; 2])> {
    let id = s.doc.find_feature(name).unwrap().id;
    let sk = s.doc.sketch(id).unwrap();
    sk.curves
        .iter()
        .filter_map(|c| match c.kind {
            solidcraft_sketch::CurveKind::Line { a, b } => {
                let (p, q) = (sk.points[a].pos, sk.points[b].pos);
                Some((c.id.clone(), [p.x, p.y], [q.x, q.y]))
            }
            _ => None,
        })
        .collect()
}

/// The id of the line of sketch `name` with both ends at x = `x`.
fn line_at_x(s: &Session, name: &str, x: f64) -> String {
    sketch_lines(s, name).into_iter().find(|(_, a, b)| (a[0] - x).abs() < 1e-9 && (b[0] - x).abs() < 1e-9).unwrap().0
}

/// The id of the line of sketch `name` with both ends at y = `y`.
fn line_at_y(s: &Session, name: &str, y: f64) -> String {
    sketch_lines(s, name).into_iter().find(|(_, a, b)| (a[1] - y).abs() < 1e-9 && (b[1] - y).abs() < 1e-9).unwrap().0
}

fn delete_line(s: &mut Session, sketch: &str, line: &str) {
    run(s, "sketch.edit", json!({"sketch": sketch}));
    run(s, "sketch.delete", json!({"entities": [line]}));
    run(s, "sketch.finish", json!({}));
}

/// Each body's bounding box by name.
fn boxes(s: &mut Session) -> std::collections::BTreeMap<String, ([f64; 3], [f64; 3])> {
    bodies(s).into_iter().map(|b| (b.0, (b.2, b.3))).collect()
}

#[test]
fn projected_edges_are_not_member_lines() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 100, "height": 10, "body_name": "Plate"}));
    // A sketch on the plate's top face projects its edges; the one line drawn is the only member.
    run(&mut s, "sketch.create", json!({"plane": {"face": [50, 50, 10]}, "name": "On plate"}));
    run(&mut s, "sketch.line", json!({"points": [[10, 10], [90, 10]]}));
    run(&mut s, "sketch.finish", json!({}));
    // The plate's four edges are in the sketch, projected.
    assert_eq!(sketch_lines(&s, "On plate").len(), 5);
    run(&mut s, "frame.member", json!({"sketch": "On plate", "profile": tube()}));
    let members: Vec<Measured> = bodies(&mut s).into_iter().filter(|b| b.0 != "Plate").collect();
    assert_eq!(members.len(), 1, "{members:?}");
    assert!(rel(members[0].1, tube_area() * 80.0) < MEASURED, "{members:?}");
}

#[test]
fn a_profile_sketch_on_a_face_is_its_drawn_section() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 100, "height": 10, "body_name": "Plate"}));
    run(&mut s, "sketch.create", json!({"plane": {"face": [50, 50, 10]}, "name": "Section on plate"}));
    run(&mut s, "sketch.circle.center", json!({"center": [0, 0], "radius": 20}));
    run(&mut s, "sketch.circle.center", json!({"center": [0, 0], "radius": 17}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "Post"}));
    run(&mut s, "sketch.line", json!({"points": [[200, 0], [200, 1000]]}));
    run(&mut s, "sketch.finish", json!({}));
    assert_eq!(sketch_lines(&s, "Section on plate").len(), 4, "the plate's edges, projected");
    run(&mut s, "frame.member", json!({"sketch": "Post", "profile_sketch": "Section on plate"}));
    let member = bodies(&mut s).into_iter().find(|b| b.0 != "Plate").unwrap();
    assert!(rel(member.1, PI * (400.0 - 289.0) * 1000.0) < MEASURED, "{member:?}");
}

#[test]
fn members_keep_their_names_when_a_line_is_deleted() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}));
    let before = boxes(&mut s);
    assert_eq!(before.len(), 4);
    let right = line_at_x(&s, "Frame", 600.0);
    delete_line(&mut s, "Frame", &right);
    let after = boxes(&mut s);
    assert_eq!(after.len(), 3, "{after:?}");
    // Each member left is still called what it was, and is where it was.
    for (name, bb) in &after {
        assert_eq!(before.get(name), Some(bb), "{name} moved: {before:?} → {after:?}");
    }
}

#[test]
fn a_renamed_member_keeps_its_name_through_line_edits() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}));
    let top = boxes(&mut s).into_iter().find(|(_, (lo, _))| (lo[1] - 380.0).abs() < 1e-6).unwrap();
    run(&mut s, "body.rename", json!({"body": top.0, "name": "Top rail"}));
    let before = boxes(&mut s);
    assert_eq!(before.get("Top rail"), Some(&top.1));
    let left = line_at_x(&s, "Frame", 0.0);
    delete_line(&mut s, "Frame", &left);
    let after = boxes(&mut s);
    assert_eq!(after.len(), 3);
    for (name, bb) in &after {
        assert_eq!(before.get(name), Some(bb), "{name}: {before:?} → {after:?}");
    }
    // It survives saving and reopening.
    let mut reopened = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    assert_eq!(boxes(&mut reopened), after);
}

#[test]
fn a_numbered_member_name_is_kept_when_the_first_feature_goes() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (bottom, top) = (line_at_y(&s, "Frame", 0.0), line_at_y(&s, "Frame", 400.0));
    run(&mut s, "frame.member", json!({"sketch": "Frame", "lines": [bottom], "profile": tube(), "name": "First"}));
    run(&mut s, "frame.member", json!({"sketch": "Frame", "lines": [top], "profile": tube(), "name": "Second"}));
    assert_eq!(boxes(&mut s).keys().collect::<Vec<_>>(), ["Rect Tube 40x40x3", "Rect Tube 40x40x3 (2)"]);
    run(&mut s, "timeline.delete", json!({"feature": "First"}));
    assert_eq!(boxes(&mut s).keys().collect::<Vec<_>>(), ["Rect Tube 40x40x3 (2)"]);
}

#[test]
fn an_asymmetric_profile_turns_with_its_3d_line() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Tower"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 1500], [600, 0, 1500]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Tower", "profile": {"kind": "bar", "width": 60, "height": 10}}));
    let bs = bodies(&mut s);
    // The post: the bar's width (its x) along world X, its height along Y.
    assert!(bs.iter().any(|b| near3(b.2, [-30.0, -5.0, 0.0]) && near3(b.3, [30.0, 5.0, 1500.0])), "{bs:?}");
    // The rail along +X: width along +Y, height up.
    assert!(bs.iter().any(|b| near3(b.2, [0.0, -30.0, 1495.0]) && near3(b.3, [600.0, 30.0, 1505.0])), "{bs:?}");
}

#[test]
fn a_post_a_hair_off_vertical_is_oriented_as_a_vertical_one() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Posts"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [1e-5, 0, 1000]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Posts", "profile": {"kind": "bar", "width": 60, "height": 10}}));
    let b = bodies(&mut s).remove(0);
    // Width along X as for a true vertical, not along Y.
    assert!((b.3[0] - b.2[0] - 60.0).abs() < 1e-3 && (b.3[1] - b.2[1] - 10.0).abs() < 1e-3, "{b:?}");
}

#[test]
fn thick_sections_build_with_their_default_radii() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rail"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [500, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    for profile in [
        json!({"kind": "angle", "width": 20, "height": 20, "thickness": 10}),
        json!({"kind": "channel", "height": 50, "width": 10, "web": 6, "flange": 2}),
        json!({"kind": "channel", "height": 20, "width": 40, "web": 5, "flange": 8}),
        json!({"kind": "i_beam", "height": 100, "width": 10, "web": 6, "flange": 5}),
    ] {
        run(&mut s, "frame.member", json!({"sketch": "Rail", "profile": profile}));
    }
    assert_eq!(bodies(&mut s).len(), 4);
}

#[test]
fn profile_dimensions_follow_parameters() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "T", "expression": "3 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rail"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [1000, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    let square = |t: f64| (1600.0 - (40.0 - 2.0 * t).powi(2)) * 1000.0;
    let sharp = json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": "T", "outer_radius": 0});
    let props = run(&mut s, "frame.profile.properties", json!({"profile": sharp}));
    assert!(rel(f(&props["area"]), square(3.0) / 1000.0) < 1e-12, "{props}");
    run(&mut s, "frame.member", json!({"sketch": "Rail", "profile": sharp}));
    assert!(rel(total_volume(&mut s), square(3.0)) < 1e-9);
    run(&mut s, "parameters.change", json!({"name": "T", "expression": "2 mm"}));
    assert!(rel(total_volume(&mut s), square(2.0)) < 1e-9, "{}", total_volume(&mut s));
    // A parameter that makes the profile not fit fails the feature, naming the dimension.
    let err = s.execute("parameters.change", &json!({"name": "T", "expression": "25 mm"})).err().map(|e| e.to_string()).unwrap_or_default();
    let fid = s.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().id;
    let failing = s.model.result(fid).and_then(|r| r.error.clone()).unwrap_or_default();
    assert!(err.contains("wall") || failing.contains("wall"), "{err} / {failing}");
}

#[test]
fn bare_profile_numbers_are_in_the_design_units() {
    let mut s = Session::default();
    run(&mut s, "document.units", json!({"units": "in"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rail"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [10, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Rail", "profile": {"kind": "bar", "width": 2, "height": 1}}));
    let b = bodies(&mut s).remove(0);
    // 2 in across (world Y) and 1 in up (world Z).
    assert!((b.3[1] - b.2[1] - 50.8).abs() < 1e-6 && (b.3[2] - b.2[2] - 25.4).abs() < 1e-6, "{b:?}");
    let props = run(&mut s, "frame.profile.properties", json!({"profile": {"kind": "round_bar", "diameter": 1}}));
    assert!((f(&props["width"]) - 25.4).abs() < 1e-9, "{props}");
}

#[test]
fn members_follow_a_frame_sketch_in_another_component() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let c = run(&mut s, "component.create", json!({"name": "Frame parts", "grounded": false}))["component"].as_u64().unwrap();
    let o = s.doc.occurrence_of(c).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence": o, "translate": [0, 0, 100], "capture": true}));
    // Made in the moved component, the members still lie on the root's frame sketch.
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 4);
    for b in &bs {
        assert!((b.2[2] + 20.0).abs() < 1e-6 && (b.3[2] - 20.0).abs() < 1e-6, "{b:?}");
    }
}

#[test]
fn listing_a_line_twice_is_refused() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let bottom = line_at_y(&s, "Frame", 0.0);
    let err = s.execute("frame.member", &json!({"sketch": "Frame", "profile": tube(), "lines": [bottom, bottom]})).unwrap_err().to_string();
    assert!(err.contains("listed twice"), "{err}");
}

/// Edit Feature keeps each member's name, a renamed one too, through a change of profile; giving
/// the members a name renames them all.
#[test]
fn editing_the_feature_keeps_member_names_until_it_names_them() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}))["feature"].clone();
    let top = boxes(&mut s).into_iter().find(|(_, (lo, _))| (lo[1] - 380.0).abs() < 1e-6).unwrap().0;
    run(&mut s, "body.rename", json!({"body": top, "name": "Top rail"}));
    let names: Vec<String> = boxes(&mut s).into_keys().collect();
    let bigger = json!({"kind": "rect_tube", "width": 50, "height": 50, "wall": 3});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": {"sketch": "Frame", "profile": bigger}}));
    let after = boxes(&mut s);
    assert_eq!(after.keys().cloned().collect::<Vec<_>>(), names);
    // The top rail is the 50 mm tube now, centred on the line at y = 400.
    assert!(near3(after["Top rail"].0, [0.0, 375.0, -25.0]) && near3(after["Top rail"].1, [600.0, 425.0, 25.0]), "{after:?}");
    let params = json!({"sketch": "Frame", "profile": bigger, "body_name": "Rail"});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": params}));
    assert_eq!(boxes(&mut s).into_keys().collect::<Vec<_>>(), ["Rail", "Rail (2)", "Rail (3)", "Rail (4)"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(boxes(&mut s).into_keys().collect::<Vec<_>>(), names);
}

/// A 300 mm post up Z from the origin of `profile`, its member body `Post`, made by the feature
/// `Post member` from the 3D sketch `Post path`.
fn capped_post(profile: Value) -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Post path"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 300]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Post path", "name": "Post member", "body_name": "Post", "profile": profile}));
    s
}

/// The face of `body` through `point` facing along `normal`, as the view picks it.
fn face_pick(s: &Session, body: &str, point: [f64; 3], normal: [f64; 3]) -> Value {
    let st = s.model.state();
    let b = st.body(body).unwrap();
    let at = solidcore_record::Vec3::new(point[0], point[1], point[2]);
    let n = solidcore_record::Vec3::new(normal[0], normal[1], normal[2]);
    let i = (0..b.body.face_count())
        .find(|i| b.body.planar_face(*i).is_some_and(|pl| pl.normal().dot(n) > 0.999 && (pl.origin - at).dot(n).abs() < 1e-6))
        .unwrap();
    json!({"body": body, "index": i, "face": point})
}

fn square_tube() -> Value {
    json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3, "outer_radius": 0})
}

fn measured(s: &mut Session, name: &str) -> Measured {
    bodies(s).into_iter().find(|b| b.0 == name).unwrap_or_else(|| panic!("no body {name}"))
}

#[test]
fn an_end_cap_is_flush_with_the_outside_unless_offset() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let r = run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6}));
    assert_eq!(r["created_bodies"], json!(["Post End Cap"]), "{r}");
    // No offset: the plate is the profile's outside, 40 x 40, 6 thick, on top.
    let cap = measured(&mut s, "Post End Cap");
    assert!(rel(cap.1, 40.0 * 40.0 * 6.0) < 1e-9, "{cap:?}");
    assert!(near3(cap.2, [-20.0, -20.0, 300.0]) && near3(cap.3, [20.0, 20.0, 306.0]), "{cap:?}");
    // The bottom end faces down: its cap hangs below the post. Offset by half its thickness of
    // 4: a 36 x 36 plate.
    let bottom = face_pick(&s, "Post", [18.5, 0.0, 0.0], [0.0, 0.0, -1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [bottom], "thickness": 4, "ratio": 0.5}));
    let cap = measured(&mut s, "Post End Cap (2)");
    assert!(near3(cap.2, [-18.0, -18.0, -4.0]) && near3(cap.3, [18.0, 18.0, 0.0]), "{cap:?}");
}

#[test]
fn an_end_cap_goes_on_a_butted_through_end() {
    for corner in ["butt", "auto"] {
        let mut s = Session::default();
        rectangle_frame(&mut s);
        run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": square_tube(), "corner": corner}));
        let (bottom, _) = bottom_and_left(&mut s);
        // The bottom member runs through: its start is cut square at x = -20, not at the line's end.
        let start = face_pick(&s, &bottom.0, [-20.0, -18.5, 0.0], [-1.0, 0.0, 0.0]);
        run(&mut s, "frame.end_cap", json!({"faces": [start], "thickness": 6}));
        let cap = measured(&mut s, &format!("{} End Cap", bottom.0));
        assert!((cap.3[0] + 20.0).abs() < 1e-6 && (cap.2[0] + 26.0).abs() < 1e-6, "{corner}: {cap:?}");
    }
}

#[test]
fn an_inward_end_cap_measures_a_member_as_cut() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": square_tube(), "corner": "butt"}));
    let (bottom, left) = bottom_and_left(&mut s);
    // The 600 member is built 640 long (20 past each end): 620 fits, though its line is 600.
    let start = face_pick(&s, &bottom.0, [-20.0, -18.5, 0.0], [-1.0, 0.0, 0.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [start], "thickness": 620, "direction": "inward"}));
    run(&mut s, "edit.undo", json!({}));
    // 640 does not fit.
    let e = s.execute("frame.end_cap", &json!({"faces": [start], "thickness": 640, "direction": "inward"})).unwrap_err().to_string();
    assert!(e.contains("needs a longer member"), "{e}");
    // The 400 member's end is butted against the bottom member: refused at the face, whatever
    // the thickness.
    let end = face_pick(&s, &left.0, [-18.5, 380.0, 0.0], [0.0, 1.0, 0.0]);
    let e = s.execute("frame.end_cap", &json!({"faces": [end], "thickness": 6, "direction": "inward"})).unwrap_err().to_string();
    assert!(e.contains("not an open end"), "{e}");
}

#[test]
fn an_end_cap_on_a_butted_or_coped_end_is_refused() {
    // Auto's U is fitted, flat on T's near face at y = 380 in square tube.
    for (corner, at) in [("butt", 380.0), ("coped_butt", 20.0), ("auto", 380.0)] {
        let mut s = Session::default();
        rectangle_frame(&mut s);
        run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": square_tube(), "corner": corner}));
        let (_, left) = bottom_and_left(&mut s);
        // U's butted end faces up at y = 380; its cope face, flat on T's near face, down at y = 20.
        let normal = if at > 200.0 { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] };
        let face = face_pick(&s, &left.0, [-18.5, at, 0.0], normal);
        for direction in ["outward", "inward"] {
            let e = s.execute("frame.end_cap", &json!({"faces": [face], "thickness": 6, "direction": direction})).unwrap_err().to_string();
            assert!(e.contains("not an open end"), "{corner} {direction}: {e}");
        }
    }
}

#[test]
fn an_end_cap_on_a_mitred_end_is_refused() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": square_tube(), "corner": "mitre"}));
    let (bottom, _) = bottom_and_left(&mut s);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let start = face_pick(&s, &bottom.0, [-19.0, -19.0, 0.0], [-h, h, 0.0]);
    let e = s.execute("frame.end_cap", &json!({"faces": [start], "thickness": 6})).unwrap_err().to_string();
    assert!(e.contains("not square"), "{e}");
}

/// A cap made while another is suppressed doesn't take that one's name: restored, each keeps its
/// own, so what refers to the second cap by name still finds it.
#[test]
fn an_end_cap_made_while_another_is_suppressed_keeps_a_name_of_its_own() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let bottom = face_pick(&s, "Post", [18.5, 0.0, 0.0], [0.0, 0.0, -1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6, "name": "Top cap"}));
    run(&mut s, "timeline.suppress", json!({"feature": "Top cap"}));
    let r = run(&mut s, "frame.end_cap", json!({"faces": [bottom], "thickness": 4, "name": "Bottom cap"}));
    assert_eq!(r["created_bodies"], json!(["Post End Cap (2)"]), "{r}");
    run(&mut s, "timeline.suppress", json!({"feature": "Top cap", "suppressed": false}));
    // The top cap on top, the bottom one below the post, each under the name it was made with.
    assert!(measured(&mut s, "Post End Cap").2[2] > 299.0);
    assert!(measured(&mut s, "Post End Cap (2)").3[2] < 1e-6);
}

#[test]
fn end_cap_corner_chamfers_cut_each_corner() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 5, "offset": 2, "chamfer": 4}));
    // 36 x 36 less four 4 x 4 corner triangles.
    let v = measured(&mut s, "Post End Cap").1;
    assert!(rel(v, (36.0 * 36.0 - 4.0 * 8.0) * 5.0) < 1e-9, "{v}");
}

#[test]
fn end_cap_on_a_rounded_tube_keeps_the_rounding_less_the_inset() {
    // Outer radius 6 (twice the wall): inset 3 leaves 34 x 34 with corners of radius 3.
    let mut s = capped_post(tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let before = s.doc.to_json();
    let e = s.execute("frame.end_cap", &json!({"faces": [top.clone()], "thickness": 6, "ratio": 0.5, "chamfer": 2})).unwrap_err().to_string();
    assert!(e.contains("at least the corner rounding"), "{e}");
    assert_eq!(s.doc.to_json(), before);
    run(&mut s, "frame.end_cap", json!({"faces": [top.clone()], "thickness": 6, "ratio": 0.5}));
    let v = measured(&mut s, "Post End Cap").1;
    assert!(rel(v, (34.0 * 34.0 - (4.0 - PI) * 9.0) * 6.0) < MEASURED, "{v}");
    run(&mut s, "edit.undo", json!({}));
    // A chamfer as long as the rounding cuts it away: four 3 x 3 triangles off the 34 square.
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6, "ratio": 0.5, "chamfer": 3}));
    let v = measured(&mut s, "Post End Cap").1;
    assert!(rel(v, (34.0 * 34.0 - 4.0 * 4.5) * 6.0) < 1e-9, "{v}");
}

#[test]
fn an_end_cap_on_an_i_beam_closes_its_whole_end_square() {
    // An I-beam is open, so its cap is the 50 x 100 rectangle round it, in by 1: 48 x 98.
    let mut s = capped_post(json!({"kind": "i_beam", "height": 100, "width": 50, "web": 4.5, "flange": 7}));
    let post = measured(&mut s, "Post");
    let mid = [(post.2[0] + post.3[0]) / 2.0, (post.2[1] + post.3[1]) / 2.0, 300.0];
    let top = face_pick(&s, "Post", mid, [0.0, 0.0, 1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6, "offset": 1}));
    let cap = measured(&mut s, "Post End Cap");
    assert!(rel(cap.1, 48.0 * 98.0 * 6.0) < 1e-9, "{cap:?}");
    // A 2 mm chamfer cuts the rectangle's four corners.
    run(&mut s, "edit.undo", json!({}));
    let top = face_pick(&s, "Post", mid, [0.0, 0.0, 1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [top.clone()], "thickness": 6, "offset": 1, "chamfer": 2}));
    let v = measured(&mut s, "Post End Cap").1;
    assert!(rel(v, (48.0 * 98.0 - 4.0 * 2.0) * 6.0) < 1e-9, "{v}");
    // An inset of half the beam's width leaves no plate, and nothing changes.
    run(&mut s, "edit.undo", json!({}));
    let before = s.doc.to_json();
    let e = s.execute("frame.end_cap", &json!({"faces": [top], "thickness": 6, "offset": 25})).unwrap_err().to_string();
    assert!(e.contains("leaves no plate"), "{e}");
    assert_eq!(s.doc.to_json(), before);
}

#[test]
fn an_inward_end_cap_cuts_the_member_back() {
    let mut s = capped_post(json!({"kind": "round_tube", "diameter": 40, "wall": 3}));
    let ring = PI * (400.0 - 289.0);
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let r = run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 4, "ratio": 0.5, "direction": "inward"}));
    assert_eq!(r["changed_bodies"], json!(["Post"]), "{r}");
    let post = measured(&mut s, "Post");
    assert!(rel(post.1, ring * 296.0) < MEASURED, "{post:?}");
    assert!((post.3[2] - 296.0).abs() < 1e-6, "{post:?}");
    // Radius 20 less an inset of 2, 4 thick, flush with the old end.
    let cap = measured(&mut s, "Post End Cap");
    assert!(rel(cap.1, PI * 18.0 * 18.0 * 4.0) < MEASURED, "{cap:?}");
    assert!((cap.2[2] - 296.0).abs() < 1e-6 && (cap.3[2] - 300.0).abs() < 1e-6, "{cap:?}");
}

/// The post's body moved into a component placed 50 along Y, capped from the top of the design:
/// the cap is authored in the design's frame, the post is stored in its component's. The inward
/// cut is made where the post is, so it is cut back by the cap's thickness, not missed.
fn a_post_in_a_placed_component() -> Session {
    let mut s = capped_post(square_tube());
    let holder = run(&mut s, "component.create", json!({"name": "Holder", "grounded": false}))["component"].as_u64().unwrap();
    run(&mut s, "occurrence.move", json!({"occurrence": "Holder:1", "translate": [0, 50, 0]}));
    run(&mut s, "component.activate", json!({"component": 0}));
    std::sync::Arc::make_mut(&mut s.doc).body_components.insert("Post".into(), holder);
    s.refresh();
    s
}

#[test]
fn an_inward_end_cap_cuts_a_member_in_a_placed_component_where_it_stands() {
    let mut s = a_post_in_a_placed_component();
    let post = measured(&mut s, "Post");
    assert!(near3(post.2, [-20.0, 30.0, 0.0]) && near3(post.3, [20.0, 70.0, 300.0]), "{post:?}");
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let top = json!({"body": top["body"], "index": top["index"], "face": [18.5, 50.0, 300.0]});
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 4, "direction": "inward"}));
    // 40 x 40 x 3 tube, 296 long after the cut; the cap 40 x 40 x 4 where the end was.
    let post = measured(&mut s, "Post");
    assert!(near3(post.2, [-20.0, 30.0, 0.0]) && near3(post.3, [20.0, 70.0, 296.0]), "{post:?}");
    assert!(rel(post.1, (1600.0 - 34.0 * 34.0) * 296.0) < 1e-9, "{post:?}");
    let cap = measured(&mut s, "Post End Cap");
    assert!(near3(cap.2, [-20.0, 30.0, 296.0]) && near3(cap.3, [20.0, 70.0, 300.0]), "{cap:?}");
}

/// Moving the occurrence a capped member is placed by rebuilds the cap: the incremental model
/// gives what a model built from scratch does.
#[test]
fn moving_a_capped_members_occurrence_rebuilds_the_cap_as_from_scratch() {
    let mut s = a_post_in_a_placed_component();
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let top = json!({"body": top["body"], "index": top["index"], "face": [18.5, 50.0, 300.0]});
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 4, "direction": "inward"}));
    run(&mut s, "occurrence.move", json!({"occurrence": "Holder:1", "translate": [30, 0, 0]}));
    let warm = bodies(&mut s);
    let mut cold = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    let cold = bodies(&mut cold);
    assert_eq!(warm.len(), cold.len(), "{warm:?} vs {cold:?}");
    for (w, c) in warm.iter().zip(&cold) {
        assert!(w.0 == c.0 && rel(w.1, c.1) < 1e-9 && near3(w.2, c.2) && near3(w.3, c.3), "{w:?} vs {c:?}");
    }
}

/// Editing a cap down to its second end leaves that cap its own name: the first end's name goes
/// with the end, so what was set on it (a material, a reference) doesn't move to the other cap.
#[test]
fn an_end_cap_edit_that_drops_an_end_keeps_each_remaining_cap_its_name() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let bottom = face_pick(&s, "Post", [18.5, 0.0, 0.0], [0.0, 0.0, -1.0]);
    let r = run(&mut s, "frame.end_cap", json!({"faces": [top, bottom.clone()], "thickness": 6, "name": "Caps"}));
    assert_eq!(r["created_bodies"], json!(["Post End Cap", "Post End Cap (2)"]), "{r}");
    run(&mut s, "timeline.redefine", json!({"feature": "Caps", "command": "frame.end_cap", "params": {"faces": [bottom], "thickness": 6}}));
    let st = s.model.state();
    assert!(st.body("Post End Cap").is_none(), "{:?}", st.bodies.iter().map(|b| &b.name).collect::<Vec<_>>());
    drop(st);
    assert!(measured(&mut s, "Post End Cap (2)").3[2] < 1e-6, "the bottom cap keeps its name");
}

/// Editing a cap from one end onto another: the new end's cap is not given the removed end's
/// name, which stays kept back as a renamed body's is.
#[test]
fn an_end_cap_edit_onto_another_end_does_not_hand_it_the_removed_ends_name() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let bottom = face_pick(&s, "Post", [18.5, 0.0, 0.0], [0.0, 0.0, -1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6, "name": "Cap"}));
    run(&mut s, "timeline.redefine", json!({"feature": "Cap", "command": "frame.end_cap", "params": {"faces": [bottom], "thickness": 6}}));
    assert!(s.model.state().body("Post End Cap").is_none());
    assert!(measured(&mut s, "Post End Cap (2)").3[2] < 1e-6, "the bottom end's cap has a name of its own");
}

/// An end added before one the cap already closes doesn't take that one's name: the kept end's
/// cap keeps it, and the new one is numbered after it.
#[test]
fn an_end_added_before_a_kept_end_leaves_that_cap_its_name() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let bottom = face_pick(&s, "Post", [18.5, 0.0, 0.0], [0.0, 0.0, -1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [bottom.clone()], "thickness": 6, "name": "Cap"}));
    run(&mut s, "timeline.redefine", json!({"feature": "Cap", "command": "frame.end_cap", "params": {"faces": [top, bottom], "thickness": 6}}));
    assert!(measured(&mut s, "Post End Cap").3[2] < 1e-6, "the bottom cap keeps its name");
    assert!(measured(&mut s, "Post End Cap (2)").2[2] > 299.0, "the new top cap is numbered after it");
}

#[test]
fn end_caps_follow_member_edits_renames_undo_and_reopen() {
    let mut s = capped_post(square_tube());
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let bottom = face_pick(&s, "Post", [18.5, 0.0, 0.0], [0.0, 0.0, -1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [top, bottom], "thickness": 6, "ratio": 0.5, "name": "Caps"}));
    run(&mut s, "body.rename", json!({"body": "Post", "name": "Column"}));
    let params = json!({"sketch": "Post path", "body_name": "Column", "profile": {"kind": "rect_tube", "width": 60, "height": 40, "wall": 3, "outer_radius": 0}});
    run(&mut s, "timeline.redefine", json!({"feature": "Post member", "command": "frame.member", "params": params}));
    let cap = measured(&mut s, "Post End Cap");
    assert!(rel(cap.1, 54.0 * 34.0 * 6.0) < 1e-9, "{cap:?}");
    assert!(near3(cap.2, [-27.0, -17.0, 300.0]), "{cap:?}");
    assert!(measured(&mut s, "Post End Cap (2)").2[2] < -5.9);
    let reopened = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    assert!(rel(reopened.model.state().body("Post End Cap").unwrap().mesh().measure().volume, 54.0 * 34.0 * 6.0) < 1e-7);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.model.state().body("Post End Cap").is_none());
    run(&mut s, "edit.redo", json!({}));
    assert!(rel(measured(&mut s, "Post End Cap").1, 34.0 * 34.0 * 6.0) < 1e-9);
}

#[test]
fn end_caps_refuse_bad_picks_and_values_atomically() {
    let mut s = capped_post(square_tube());
    run(&mut s, "solid.box", json!({"corner": [100, 0, 0], "length": 10, "width": 10, "height": 10, "body_name": "Block"}));
    let top = face_pick(&s, "Post", [18.5, 0.0, 300.0], [0.0, 0.0, 1.0]);
    let side = face_pick(&s, "Post", [20.0, 0.0, 150.0], [1.0, 0.0, 0.0]);
    let block = face_pick(&s, "Block", [105.0, 5.0, 10.0], [0.0, 0.0, 1.0]);
    // The same end face picked at another point on it is still the same end.
    let top_again = face_pick(&s, "Post", [0.0, 18.5, 300.0], [0.0, 0.0, 1.0]);
    let good = json!({"faces": [top.clone()], "thickness": 6});
    let cases = [
        (json!({"faces": [side]}), "not an end"),
        (json!({"faces": [block]}), "not on a frame member"),
        (json!({"faces": [top.clone(), top.clone()]}), "twice"),
        (json!({"faces": [top, top_again.clone()]}), "twice"),
        (json!({"faces": [top_again.clone(), top_again], "direction": "inward"}), "twice"),
        (json!({"faces": []}), ""),
        (json!({"offset": 20}), "leaves no plate"),
        (json!({"ratio": 4}), "leaves no plate"),
        (json!({"offset": 1, "ratio": 0.5}), "not both"),
        (json!({"chamfer": 25}), "too large"),
        (json!({"thickness": -1}), "thickness"),
        (json!({"thickness": "1e99 mm"}), ""),
        (json!({"thickness": 0}), "thickness"),
        (json!({"ratio": "-1"}), "ratio"),
        (json!({"direction": "sideways"}), ""),
        (json!({"direction": "inward", "thickness": 300, "offset": 0}), "longer member"),
        (json!({"unsupported": true}), ""),
    ];
    for (change, why) in cases {
        let mut p = good.clone();
        for (k, v) in change.as_object().unwrap() {
            p[k] = v.clone();
        }
        let before = s.doc.to_json();
        let e = s.execute("frame.end_cap", &p).map(|_| ()).unwrap_err().to_string();
        assert!(e.contains(why), "{p}: {e}");
        assert_eq!(s.doc.to_json(), before, "{p}");
    }
    // An angle is open too: its cap is the 40 x 40 square round it, in by 1 (half of 2).
    let mut s = capped_post(json!({"kind": "angle", "width": 40, "height": 40, "thickness": 4}));
    let end = face_pick(&s, "Post", [-18.0, -19.0, 300.0], [0.0, 0.0, 1.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [end], "thickness": 2, "ratio": 0.5}));
    let v = measured(&mut s, "Post End Cap").1;
    assert!(rel(v, 38.0 * 38.0 * 2.0) < 1e-9, "{v}");
}

/// Members longer than 810 mm mirrored as bodies measure as their sources, across a named plane
/// and across a plane by origin and normal: SMLib's mirror gave each straight edge as a B-spline
/// 1,234,567 edge lengths long, which the copy's measure refused ("invalid B-rep record: flat
/// arrays", #14413, #15379).
#[test]
fn long_members_mirrored_as_bodies_measure_as_their_sources() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rails"}));
    run(&mut s, "sketch.line", json!({"points": [[100, 0], [100, 1100]]}));
    run(&mut s, "sketch.line", json!({"points": [[300, -200], [300, 1800]]}));
    run(&mut s, "sketch.finish", json!({}));
    let channel = json!({"kind": "channel", "height": 152.4, "width": 76.2, "web": 6.4, "flange": 9, "root_radius": 12.2, "toe_radius": 2.4});
    run(&mut s, "frame.member", json!({"sketch": "Rails", "lines": ["l1"], "profile": channel, "body_name": "Channel"}));
    run(&mut s, "frame.member", json!({"sketch": "Rails", "lines": ["l2"], "profile": tube(), "body_name": "Tube"}));
    run(&mut s, "solid.mirror", json!({"bodies": ["Channel", "Tube"], "plane": "YZ"}));
    run(&mut s, "solid.mirror", json!({"bodies": ["Channel"], "plane": {"origin": [1000, 0, 0], "normal": [1, 0, 0]}}));
    let bs = bodies(&mut s);
    let get = |name: &str| bs.iter().find(|b| b.0 == name).cloned().unwrap_or_else(|| panic!("no {name} in {bs:?}"));
    for (source, copy, x0) in [("Channel", "Channel (2)", 0.0), ("Tube", "Tube (2)", 0.0), ("Channel", "Channel (3)", 1000.0)] {
        let (a, b) = (get(source), get(copy));
        assert!(rel(b.1, a.1) < 1e-9, "{copy}: volume {} against {}", b.1, a.1);
        // Mirrored in x = x0: x runs the other way, y and z stay.
        assert!(near3(b.2, [2.0 * x0 - a.3[0], a.2[1], a.2[2]]) && near3(b.3, [2.0 * x0 - a.2[0], a.3[1], a.3[2]]), "{copy}: {b:?} from {a:?}");
    }
}

/// A frame member feature mirrors and patterns as a feature: each copy is every member it made,
/// placed by the mirror or the pattern, and the copies follow the member's lines. Mirroring the
/// feature was refused ("cannot be patterned or mirrored yet").
#[test]
fn a_member_feature_mirrors_and_patterns_and_follows_its_lines() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "L", "expression": "700 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rails"}));
    run(&mut s, "sketch.line", json!({"points": [[100, 0], [100, "L"]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Rails", "profile": square_tube(), "name": "Rail member", "body_name": "Rail"}));
    run(&mut s, "solid.mirror", json!({"features": ["Rail member"], "plane": "YZ"}));
    run(&mut s, "solid.pattern.rectangular", json!({"features": ["Rail member"], "dir1": [0, 0, 1], "count1": 2, "spacing1": 300}));
    let check = |s: &mut Session, length: f64| {
        let bs = bodies(s);
        assert_eq!(bs.len(), 3, "{bs:?}");
        for b in &bs {
            assert!(rel(b.1, 444.0 * length) < 1e-9, "{b:?}");
        }
        let at = |lo: [f64; 3], hi: [f64; 3]| bs.iter().any(|b| near3(b.2, lo) && near3(b.3, hi));
        assert!(at([80.0, 0.0, -20.0], [120.0, length, 20.0]), "the rail: {bs:?}");
        assert!(at([-120.0, 0.0, -20.0], [-80.0, length, 20.0]), "its mirror in x = 0: {bs:?}");
        assert!(at([80.0, 0.0, 280.0], [120.0, length, 320.0]), "its copy 300 up: {bs:?}");
    };
    check(&mut s, 700.0);
    run(&mut s, "parameters.change", json!({"name": "L", "expression": "760 mm"}));
    check(&mut s, 760.0);
}

// ---------------------------------------------------------------------------------------------
// Members between pairs of points (#15705)

/// A 600 × 400 × 400 box `Box` (feature `Box`) from the rectangle `Base` on XY: its corners are
/// the points the pairs join.
fn box_frame(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(s, "sketch.finish", json!({}));
    run(s, "solid.extrude", json!({"distance": 400, "name": "Box", "body_name": "Box"}));
}

/// The upright at the box's corner (600, 0), as `pairs` takes it.
fn upright() -> Value {
    json!({"a": {"vertex": [600, 0, 0]}, "b": {"vertex": [600, 0, 400]}})
}

/// The rail along the box's top front edge, as `pairs` takes it.
fn top_rail() -> Value {
    json!({"a": {"vertex": [0, 0, 400]}, "b": {"vertex": [600, 0, 400]}})
}

/// The bounding boxes of the bodies other than the box, by name.
fn member_boxes(s: &mut Session) -> std::collections::BTreeMap<String, ([f64; 3], [f64; 3])> {
    boxes(s).into_iter().filter(|(name, _)| name != "Box").collect()
}

fn has_box(boxes: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>, lo: [f64; 3], hi: [f64; 3]) -> bool {
    boxes.values().any(|(l, h)| near3(*l, lo) && near3(*h, hi))
}

/// The kind of the feature `id` (as a command's reply gives it).
fn member_kind(s: &Session, id: &Value) -> solidcraft_doc::FeatureKind {
    s.doc.feature(id.as_u64().unwrap()).unwrap().kind.clone()
}

#[test]
fn members_between_point_pairs() {
    let mut s = Session::default();
    box_frame(&mut s);
    let r = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}));
    let members = member_boxes(&mut s);
    assert_eq!(members.len(), 2, "{members:?}");
    // The upright: square to Z, the profile's x along world X.
    assert!(has_box(&members, [580.0, -20.0, 0.0], [620.0, 20.0, 400.0]), "{members:?}");
    // The rail along +X: the profile's x along +Y, its y up.
    assert!(has_box(&members, [0.0, -20.0, 380.0], [600.0, 20.0, 420.0]), "{members:?}");
    let solidcraft_doc::FeatureKind::FrameMember { path_sketch, pairs, next_pair, .. } = member_kind(&s, &r["feature"]) else { panic!() };
    assert_eq!(path_sketch, None);
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair1", "pair2"]);
    assert_eq!(next_pair, 3);
}

#[test]
fn point_pairs_and_sketch_lines_in_one_frame_member() {
    let mut s = Session::default();
    box_frame(&mut s);
    let bottom = line_at_y(&s, "Base", 0.0);
    let r = run(&mut s, "frame.member", json!({"sketch": "Base", "lines": [bottom], "profile": tube(), "pairs": [upright()]}));
    let members = member_boxes(&mut s);
    assert_eq!(members.len(), 2, "{members:?}");
    assert!(has_box(&members, [0.0, -20.0, -20.0], [600.0, 20.0, 20.0]), "{members:?}");
    assert!(has_box(&members, [580.0, -20.0, 0.0], [620.0, 20.0, 400.0]), "{members:?}");
    let ids = solidcraft_doc::frame::member_line_ids(&s.model.state(), &member_kind(&s, &r["feature"])).unwrap();
    assert_eq!(ids, [bottom, "pair1".to_string()]);
}

#[test]
fn a_pair_member_follows_its_point() {
    let mut s = Session::default();
    box_frame(&mut s);
    run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright()]}));
    run(&mut s, "timeline.edit", json!({"feature": "Box", "set": {"extent.distance": "500"}}));
    let members = member_boxes(&mut s);
    assert!(has_box(&members, [580.0, -20.0, 0.0], [620.0, 20.0, 500.0]), "the upright reaches the box's new top: {members:?}");
}

/// A pair on sketch points follows them when the sketch is edited.
#[test]
fn a_pair_member_follows_its_sketch_points() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Marks"}));
    run(&mut s, "sketch.point", json!({"point": [0, -100], "id": "m1"}));
    run(&mut s, "sketch.point", json!({"point": [600, -100], "id": "m2"}));
    run(&mut s, "sketch.finish", json!({}));
    let pair = json!({"a": {"sketch": "Marks", "point": "m1"}, "b": {"sketch": "Marks", "point": "m2"}});
    run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [pair]}));
    let members = member_boxes(&mut s);
    assert!(has_box(&members, [0.0, -120.0, -20.0], [600.0, -80.0, 20.0]), "{members:?}");
    run(&mut s, "sketch.edit", json!({"sketch": "Marks"}));
    run(&mut s, "sketch.move_point", json!({"point": "m2", "to": [800, -100]}));
    run(&mut s, "sketch.finish", json!({}));
    let members = member_boxes(&mut s);
    assert!(has_box(&members, [0.0, -120.0, -20.0], [800.0, -80.0, 20.0]), "{members:?}");
}

#[test]
fn a_pair_member_keeps_its_name_when_another_pair_goes() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}))["feature"].clone();
    let solidcraft_doc::FeatureKind::FrameMember { names, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(names.keys().collect::<Vec<_>>(), ["pair1", "pair2"]);
    let rail = member_boxes(&mut s).into_iter().find(|(_, (lo, _))| (lo[2] - 380.0).abs() < 1e-6).unwrap();
    assert_eq!(names["pair2"], rail.0);
    let mut kept = top_rail();
    kept["id"] = json!("pair2");
    let params = json!({"profile": tube(), "pairs": [kept], "next_pair": 3});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": params}));
    let after = member_boxes(&mut s);
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(after.get(&rail.0), Some(&rail.1), "the rail keeps its name and place: {after:?}");
    let solidcraft_doc::FeatureKind::FrameMember { pairs, next_pair, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair2"]);
    assert_eq!(next_pair, 3);
}

#[test]
fn an_end_cap_closes_a_pair_member() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": square_tube(), "pairs": [upright()], "body_name": "Post"}))["feature"].clone();
    let top = face_pick(&s, "Post", [618.5, 0.0, 400.0], [0.0, 0.0, 1.0]);
    let r = run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6}));
    let cap = measured(&mut s, "Post End Cap");
    assert!(near3(cap.2, [580.0, -20.0, 400.0]) && near3(cap.3, [620.0, 20.0, 406.0]), "{cap:?}");
    let solidcraft_doc::FeatureKind::FrameEndCap { ends, .. } = member_kind(&s, &r["feature"]) else { panic!() };
    assert_eq!(ends[0].member.line, "pair1");
    assert_eq!(ends[0].member.feature.0, fid.as_u64().unwrap());
}

#[test]
fn pairs_are_refused_when() {
    let mut s = Session::default();
    box_frame(&mut s);
    let same = json!({"a": {"vertex": [600, 0, 0]}, "b": {"vertex": [600, 0, 0]}});
    let fixed = json!({"a": {"point": [0, 0, 0]}, "b": {"vertex": [600, 0, 400]}});
    let edge = json!({"a": {"vertex": [600, 0, 0]}, "b": {"edge": [300, 0, 0]}});
    let many: Vec<Value> = (0..=1000).map(|_| upright()).collect();
    let with_id = |pair: Value, id: &str| {
        let mut pair = pair;
        pair["id"] = json!(id);
        pair
    };
    for (p, why) in [
        (json!({"profile": tube(), "pairs": [same]}), "no length"),
        (json!({"profile": tube(), "pairs": [fixed]}), "`pairs[0].a` must be a point"),
        (json!({"profile": tube(), "pairs": [upright(), edge]}), "`pairs[1].b` must be a point"),
        (json!({"profile": tube(), "pairs": many}), "at most 1000"),
        (json!({"profile": tube()}), "`sketch`"),
        (json!({"profile": tube(), "pairs": []}), "`pairs`"),
        (json!({"profile": tube(), "pairs": "pair1"}), "`pairs`"),
        (json!({"profile": tube(), "pairs": [{"a": {"vertex": [600, 0, 0]}}]}), "`pairs[0].b`"),
        (json!({"profile": tube(), "pairs": [with_id(upright(), "pair9")], "next_pair": 3}), "pair9"),
        (json!({"profile": tube(), "pairs": [with_id(upright(), "pair1"), with_id(top_rail(), "pair1")], "next_pair": 3}), "pair1"),
        (json!({"profile": tube(), "pairs": [with_id(upright(), "rail")]}), "rail"),
        (json!({"profile": tube(), "pairs": [upright()], "next_pair": -1}), "`next_pair`"),
    ] {
        let err = s.execute("frame.member", &p).unwrap_err().to_string();
        assert!(err.contains(why), "{why}: {err}");
    }
    assert!(member_boxes(&mut s).is_empty(), "nothing was made");
}

#[test]
fn a_pair_whose_point_is_gone_fails_the_feature_naming_it() {
    let mut s = Session::default();
    box_frame(&mut s);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Marks"}));
    run(&mut s, "sketch.point", json!({"point": [600, 0], "id": "m1"}));
    run(&mut s, "sketch.finish", json!({}));
    let on_mark = json!({"a": {"sketch": "Marks", "point": "m1"}, "b": {"vertex": [600, 0, 400]}});
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [on_mark, top_rail()]}))["feature"].as_u64().unwrap();
    let before = member_boxes(&mut s);
    assert_eq!(before.len(), 2, "{before:?}");
    run(&mut s, "sketch.edit", json!({"sketch": "Marks"}));
    run(&mut s, "sketch.delete", json!({"entities": ["m1"]}));
    run(&mut s, "sketch.finish", json!({}));
    let failing = s.model.result(fid).and_then(|r| r.error.clone()).unwrap_or_default();
    assert!(failing.contains("pair1"), "{failing}");
    assert!(member_boxes(&mut s).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(member_boxes(&mut s), before);
}

#[test]
fn a_pair_member_in_a_rotated_component_turns_like_a_line_member() {
    let mut s = Session::default();
    let c = run(&mut s, "component.create", json!({"name": "Tilted", "grounded": false}))["component"].as_u64().unwrap();
    let o = s.doc.occurrence_of(c).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence": o, "axis": [1, 0, 0], "angle": "30 deg", "capture": true}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Path"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    let wire = run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [600, 0, 0]]}))["created"]["wires"][0].as_str().unwrap().to_string();
    run(&mut s, "sketch.finish", json!({}));
    let bar = json!({"kind": "bar", "width": 60, "height": 10});
    run(&mut s, "frame.member", json!({"sketch": "Path", "profile": bar, "body_name": "Line"}));
    let pair = json!({"a": {"sketch": "Path", "point": format!("{wire}:0")}, "b": {"sketch": "Path", "point": format!("{wire}:1")}});
    run(&mut s, "frame.member", json!({"profile": bar, "pairs": [pair], "body_name": "Pair"}));
    let (line, pair) = (measured(&mut s, "Line"), measured(&mut s, "Pair"));
    // Tilted 30° about X with its component: the bar's 60 mm width rises 30 mm out of the XY plane.
    assert!(line.3[2] - line.2[2] > 30.0, "{line:?}");
    assert!(near3(line.2, pair.2) && near3(line.3, pair.3), "{line:?} vs {pair:?}");
}

#[test]
fn a_pasted_frame_member_with_pairs_lands_with_its_source() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}))["feature"].clone();
    let source: Vec<_> = member_boxes(&mut s).into_values().collect();
    run(&mut s, "timeline.copy", json!({"features": [fid]}));
    run(&mut s, "timeline.paste", json!({}));
    let all: Vec<_> = member_boxes(&mut s).into_values().collect();
    assert_eq!(all.len(), 4, "{all:?}");
    for (lo, hi) in &source {
        assert_eq!(all.iter().filter(|(l, h)| near3(*l, *lo) && near3(*h, *hi)).count(), 2, "{all:?}");
    }
}

/// The stored `pairs` of a frame member, as JSON.
fn stored_pairs(s: &Session, id: &Value) -> Value {
    serde_json::to_value(&s.doc.feature(id.as_u64().unwrap()).unwrap().kind).unwrap()["pairs"].clone()
}

#[test]
fn editing_a_frame_member_keeps_pair_ids_and_never_reuses_one() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}))["feature"].clone();
    let before = stored_pairs(&s, &fid);
    let solidcraft_doc::FeatureKind::FrameMember { names, .. } = member_kind(&s, &fid) else { panic!() };
    // timeline.edit: the feature is edited in place, so a removed pair's id stays spent.
    run(&mut s, "timeline.edit", json!({"feature": fid, "set": {"pairs": [before[1].clone()]}}));
    let solidcraft_doc::FeatureKind::FrameMember { pairs, next_pair, names: after, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair2"]);
    assert_eq!(serde_json::to_value(&pairs[0]).unwrap(), before[1]);
    assert_eq!(next_pair, 3);
    assert_eq!(after["pair2"], names["pair2"]);
    // timeline.redefine without `next_pair`: the same.
    let kept = json!({"id": "pair2", "a": top_rail()["a"], "b": top_rail()["b"]});
    let params = json!({"profile": tube(), "pairs": [kept, upright()]});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": params}));
    let solidcraft_doc::FeatureKind::FrameMember { pairs, next_pair, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair2", "pair3"]);
    assert_eq!(next_pair, 4);
    // Dropping the newest pair and adding another still never reuses `pair3`.
    let params = json!({"profile": tube(), "pairs": [{"id": "pair2", "a": top_rail()["a"], "b": top_rail()["b"]}]});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": params}));
    let params = json!({"profile": tube(), "pairs": [{"id": "pair2", "a": top_rail()["a"], "b": top_rail()["b"]}, upright()]});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": params}));
    let solidcraft_doc::FeatureKind::FrameMember { pairs, next_pair, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair2", "pair4"]);
    assert_eq!(next_pair, 5);
}

#[test]
fn a_member_id_used_twice_is_refused() {
    let mut s = Session::default();
    box_frame(&mut s);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Rails"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [0, 500]], "ids": ["pair1"]}));
    run(&mut s, "sketch.finish", json!({}));
    let e =
        s.execute("frame.member", &json!({"sketch": "Rails", "lines": ["pair1"], "profile": tube(), "pairs": [upright()]})).unwrap_err().to_string();
    assert!(e.contains("`pair1` is used twice"), "{e}");
}

/// The rail along the box's bottom front edge, as `pairs` takes it.
fn bottom_rail() -> Value {
    json!({"a": {"vertex": [0, 0, 0]}, "b": {"vertex": [600, 0, 0]}})
}

fn remove(s: &mut Session, members: Value) -> Value {
    run(s, "frame.remove_member", json!({ "members": members }))
}

#[test]
fn removing_a_pair_member_takes_out_its_pair() {
    let mut s = Session::default();
    box_frame(&mut s);
    // The upright and the top rail share the corner (600, 0, 400).
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}))["feature"].clone();
    let solidcraft_doc::FeatureKind::FrameMember { names, .. } = member_kind(&s, &fid) else { panic!() };
    let rail = member_boxes(&mut s).into_iter().find(|(name, _)| *name == names["pair2"]).unwrap();
    let stored = stored_pairs(&s, &fid);
    let r = remove(&mut s, json!([{"feature": fid, "line": "pair1"}]));
    assert_eq!(r["feature"], fid, "{r}");
    let solidcraft_doc::FeatureKind::FrameMember { pairs, names: after, next_pair, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair2"]);
    assert_eq!(stored_pairs(&s, &fid), json!([stored[1]]), "pair2, the shared point with it, is as it was");
    assert_eq!(after.keys().collect::<Vec<_>>(), ["pair2"]);
    assert_eq!(after["pair2"], names["pair2"]);
    assert_eq!(next_pair, 3, "a removed pair's id is never given again");
    let members = member_boxes(&mut s);
    assert_eq!(members.len(), 1, "{members:?}");
    assert_eq!(members.get(&rail.0), Some(&rail.1), "the rail keeps its name and place: {members:?}");
}

#[test]
fn removing_a_line_member_lists_the_other_lines() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube()}))["feature"].clone();
    let before = boxes(&mut s);
    let bottom = line_at_y(&s, "Frame", 0.0);
    remove(&mut s, json!([{"feature": fid, "line": bottom}]));
    let solidcraft_doc::FeatureKind::FrameMember { lines, names, path_sketch, .. } = member_kind(&s, &fid) else { panic!() };
    let others: Vec<String> = sketch_lines(&s, "Frame").into_iter().map(|l| l.0).filter(|id| *id != bottom).collect();
    assert_eq!(lines, others);
    assert!(path_sketch.is_some());
    assert!(!names.contains_key(&bottom), "{names:?}");
    let after = boxes(&mut s);
    assert_eq!(after.len(), 3, "{after:?}");
    for (name, bb) in &after {
        assert_eq!(before.get(name), Some(bb), "{name} moved or was renamed: {before:?} → {after:?}");
    }
    assert_eq!(sketch_lines(&s, "Frame").len(), 4, "the line stays in its sketch");
}

#[test]
fn removing_the_last_member_deletes_the_frame_member() {
    let mut s = Session::default();
    box_frame(&mut s);
    let made = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright()]}));
    let fid = made["feature"].clone();
    let kind = member_kind(&s, &fid);
    let undo_steps = s.undo.len();
    let r = remove(&mut s, json!([{"feature": made["name"], "line": "pair1"}]));
    assert_eq!(r["deleted"], made["name"], "{r}");
    assert_eq!(s.undo.len(), undo_steps + 1, "one undo step");
    assert!(s.doc.feature(fid.as_u64().unwrap()).is_none());
    assert!(member_boxes(&mut s).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(member_kind(&s, &fid), kind, "one undo step brings it back, its member and name too");
    assert_eq!(member_boxes(&mut s).len(), 1);
}

#[test]
fn removing_every_line_of_a_mixed_frame_member_keeps_its_pairs() {
    let mut s = Session::default();
    box_frame(&mut s);
    let bottom = line_at_y(&s, "Base", 0.0);
    let fid = run(&mut s, "frame.member", json!({"sketch": "Base", "lines": [bottom], "profile": tube(), "pairs": [upright()]}))["feature"].clone();
    remove(&mut s, json!([{"feature": fid, "line": bottom}]));
    let solidcraft_doc::FeatureKind::FrameMember { lines, path_sketch, pairs, .. } = member_kind(&s, &fid) else { panic!() };
    assert!(lines.is_empty() && path_sketch.is_none(), "no lines left, not every line: {lines:?} {path_sketch:?}");
    assert_eq!(pairs.len(), 1);
    assert_eq!(member_boxes(&mut s).len(), 1);
}

#[test]
fn undo_and_redo_of_remove_tube() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}))["feature"].clone();
    let before = member_kind(&s, &fid);
    let undo_steps = s.undo.len();
    remove(&mut s, json!([{"feature": fid, "line": "pair1"}]));
    assert_eq!(s.undo.len(), undo_steps + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(member_kind(&s, &fid), before, "both members, pair1's name back");
    assert_eq!(member_boxes(&mut s).len(), 2);
    run(&mut s, "edit.redo", json!({}));
    let solidcraft_doc::FeatureKind::FrameMember { pairs, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.len(), 1);
    assert_eq!(member_boxes(&mut s).len(), 1);
}

#[test]
fn an_end_cap_on_a_removed_member_reports_a_broken_reference() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid =
        run(&mut s, "frame.member", json!({"profile": square_tube(), "pairs": [upright(), bottom_rail()], "body_name": "Post"}))["feature"].clone();
    let top = face_pick(&s, "Post", [618.5, 0.0, 400.0], [0.0, 0.0, 1.0]);
    let cap = run(&mut s, "frame.end_cap", json!({"faces": [top], "thickness": 6}))["feature"].as_u64().unwrap();
    remove(&mut s, json!([{"feature": fid, "line": "pair1"}]));
    assert!(s.doc.feature(cap).is_some(), "the End Cap is not deleted");
    let error = s.model.result(cap).and_then(|r| r.error.clone());
    assert!(error.is_some(), "the End Cap shows its broken reference");
}

#[test]
fn remove_member_refuses() {
    let mut s = Session::default();
    box_frame(&mut s);
    let a = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright()]}))["feature"].clone();
    let b = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [top_rail()]}))["feature"].clone();
    let doc = s.doc.clone();
    let undo_steps = s.undo.len();
    let many: Vec<Value> = (0..=1000).map(|_| json!({"feature": a, "line": "pair1"})).collect();
    for (p, why) in [
        (json!({"members": [{"feature": "Nowhere", "line": "pair1"}]}), "Nowhere"),
        (json!({"members": [{"feature": "Box", "line": "pair1"}]}), "not a Frame Member"),
        (json!({"members": [{"feature": a, "line": "pair9"}]}), "pair9"),
        (json!({"members": [{"feature": a, "line": "pair1"}, {"feature": b, "line": "pair1"}]}), "one Frame Member"),
        (json!({"members": []}), "`members`"),
        (json!({}), "`members`"),
        (json!({"members": many}), "1000"),
        (json!({"members": [{"feature": a}]}), "`members[0]`"),
    ] {
        let err = s.execute("frame.remove_member", &p).unwrap_err().to_string();
        assert!(err.contains(why), "{why}: {err}");
    }
    assert_eq!(serde_json::to_value(&*s.doc).unwrap(), serde_json::to_value(&*doc).unwrap(), "nothing changed");
    assert_eq!(s.undo.len(), undo_steps);
}

/// The bounding boxes of the bodies feature `fid` made, by name.
fn feature_boxes(s: &mut Session, fid: u64) -> std::collections::BTreeMap<String, ([f64; 3], [f64; 3])> {
    let made: Vec<String> = s.model.state().bodies.iter().filter(|b| b.feature == fid).map(|b| b.name.clone()).collect();
    boxes(s).into_iter().filter(|(name, _)| made.contains(name)).collect()
}

/// A pasted Frame Member names its tubes by position: removing one, by Remove Tube or by Edit
/// Feature, leaves every other tube its name, so what refers to them stays on them.
#[test]
fn removing_a_tube_from_a_pasted_frame_member_keeps_the_other_tubes_names() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail(), bottom_rail()]}))["feature"].clone();
    run(&mut s, "timeline.copy", json!({"features": [fid]}));
    let copy = run(&mut s, "timeline.paste", json!({}))["pasted"][0]["id"].as_u64().unwrap();
    assert!(!s.doc.feature(copy).unwrap().body_names.is_empty(), "a copy names its bodies by position");
    let before = feature_boxes(&mut s, copy);
    assert_eq!(before.len(), 3, "{before:?}");
    remove(&mut s, json!([{"feature": copy, "line": "pair1"}]));
    let after = feature_boxes(&mut s, copy);
    assert_eq!(after.len(), 2, "{after:?}");
    for (name, bb) in &after {
        assert_eq!(before.get(name), Some(bb), "{name} moved or was renamed: {before:?} → {after:?}");
    }
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(feature_boxes(&mut s, copy), before);
    // Edit Feature dropping the first pair: the same.
    let kept = |pair: Value, id: &str| {
        let mut pair = pair;
        pair["id"] = json!(id);
        pair
    };
    let params = json!({"profile": tube(), "pairs": [kept(top_rail(), "pair2"), kept(bottom_rail(), "pair3")]});
    run(&mut s, "timeline.redefine", json!({"feature": copy, "command": "frame.member", "params": params}));
    let after = feature_boxes(&mut s, copy);
    assert_eq!(after.len(), 2, "{after:?}");
    for (name, bb) in &after {
        assert_eq!(before.get(name), Some(bb), "{name} moved or was renamed: {before:?} → {after:?}");
    }
}

/// A removed pair's id is never given again: not by an Edit Feature that names a lower
/// `next_pair`, not by a new Frame Member that names ids, and `timeline.edit` can't lower it.
#[test]
fn a_removed_pair_id_cannot_be_brought_back() {
    let mut s = Session::default();
    box_frame(&mut s);
    let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright(), top_rail()]}))["feature"].clone();
    remove(&mut s, json!([{"feature": fid, "line": "pair1"}]));
    let kept = json!({"id": "pair2", "a": top_rail()["a"], "b": top_rail()["b"]});
    let params = json!({"profile": tube(), "pairs": [kept, upright()], "next_pair": 1});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.member", "params": params}));
    let solidcraft_doc::FeatureKind::FrameMember { pairs, next_pair, .. } = member_kind(&s, &fid) else { panic!() };
    assert_eq!(pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["pair2", "pair3"]);
    assert_eq!(next_pair, 4);
    let mut given = upright();
    given["id"] = json!("pair1");
    let e = s.execute("frame.member", &json!({"profile": tube(), "pairs": [given]})).unwrap_err().to_string();
    assert!(e.contains("given by the command"), "{e}");
    let doc = s.doc.clone();
    let mut past = stored_pairs(&s, &fid)[1].clone();
    past["id"] = json!("pair4");
    for set in [json!({"next_pair": 2}), json!({"next_pair": 3}), json!({"pairs": [past]})] {
        let e = s.execute("timeline.edit", &json!({"feature": fid, "set": set})).unwrap_err().to_string();
        assert!(e.contains("next_pair"), "{set}: {e}");
    }
    // Nor by naming it: neither edit gives the removed `pair1` to another tube.
    let mut revived = stored_pairs(&s, &fid)[1].clone();
    revived["id"] = json!("pair1");
    let e = s.execute("timeline.edit", &json!({"feature": fid, "set": {"pairs": [revived]}})).unwrap_err().to_string();
    assert!(e.contains("`pair1` was removed"), "{e}");
    let params = json!({"profile": tube(), "pairs": [{"id": "pair1", "a": upright()["a"], "b": upright()["b"]}]});
    let e = s.execute("timeline.redefine", &json!({"feature": fid, "command": "frame.member", "params": params})).unwrap_err().to_string();
    assert!(e.contains("`pair1` was removed"), "{e}");
    assert_eq!(serde_json::to_value(&*s.doc).unwrap(), serde_json::to_value(&*doc).unwrap(), "nothing changed");
}

/// A pair on a vertex of a body in another component follows that component when its
/// occurrence moves: the cached member is not served at its old place.
#[test]
fn a_pair_member_follows_a_vertex_in_a_moved_component() {
    let mut s = Session::default();
    let b = run(&mut s, "component.create", json!({"name": "B", "grounded": false}))["component"].as_u64().unwrap();
    box_frame(&mut s);
    run(&mut s, "component.activate", json!({"component": 0}));
    run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [upright()]}));
    let members = member_boxes(&mut s);
    assert!(has_box(&members, [580.0, -20.0, 0.0], [620.0, 20.0, 400.0]), "{members:?}");
    let o = s.doc.occurrence_of(b).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence": o, "translate": [0, 0, 100]}));
    let members = member_boxes(&mut s);
    assert!(has_box(&members, [580.0, -20.0, 100.0], [620.0, 20.0, 500.0]), "the upright moved with B: {members:?}");
    let mut cold = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    assert_eq!(member_boxes(&mut cold), members, "as a model built from scratch");
}

/// In a rotated component a pair between two body vertices comes out as the 3D sketch line
/// member between the same points does, turned the same way.
#[test]
fn a_vertex_pair_in_a_rotated_component_turns_like_a_line_member() {
    let mut s = Session::default();
    let c = run(&mut s, "component.create", json!({"name": "Tilted", "grounded": false}))["component"].as_u64().unwrap();
    box_frame(&mut s);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Path"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 400], [600, 0, 400]]}));
    run(&mut s, "sketch.finish", json!({}));
    let bar = json!({"kind": "bar", "width": 60, "height": 10});
    run(&mut s, "frame.member", json!({"sketch": "Path", "profile": bar, "body_name": "Line"}));
    run(&mut s, "frame.member", json!({"profile": bar, "pairs": [top_rail()], "body_name": "Pair"}));
    let o = s.doc.occurrence_of(c).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence": o, "axis": [1, 0, 0], "angle": "30 deg"}));
    let (line, pair) = (measured(&mut s, "Line"), measured(&mut s, "Pair"));
    // Tilted 30° about X with its component: the bar's 60 mm width rises 30 mm out of the XY plane.
    assert!(line.3[2] - line.2[2] > 30.0, "{line:?}");
    assert!(near3(line.2, pair.2) && near3(line.3, pair.3), "{line:?} vs {pair:?}");
}

// ---------------------------------------------------------------------------------------------
// Corners of members between pairs of points (#4003, #15705)

/// The frame members' measures, the box left out.
fn members_measured(s: &mut Session) -> Vec<Measured> {
    bodies(s).into_iter().filter(|b| b.0 != "Box").collect()
}

/// The member whose bounding box is `lo`..`hi`.
fn member_at(bs: &[Measured], lo: [f64; 3], hi: [f64; 3]) -> Measured {
    bs.iter().find(|b| near3(b.2, lo) && near3(b.3, hi)).unwrap_or_else(|| panic!("{lo:?}..{hi:?} in {bs:?}")).clone()
}

#[test]
fn two_pair_members_meeting_at_a_point_are_mitred() {
    let mut s = Session::default();
    box_frame(&mut s);
    // The top rail and the upright end together at (600, 0, 400).
    run(&mut s, "frame.member", json!({"profile": tube(), "pairs": [top_rail(), upright()], "corner": "mitre"}));
    let bs = members_measured(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    // Each runs out to the corner's outer side, where the 45° mitre meets x = 620 and z = 420, and
    // keeps its centreline's volume.
    let rail = member_at(&bs, [0.0, -20.0, 380.0], [620.0, 20.0, 420.0]);
    assert!(rel(rail.1, tube_area() * 600.0) < MEASURED, "{rail:?} vs {}", tube_area() * 600.0);
    let post = member_at(&bs, [580.0, -20.0, 0.0], [620.0, 20.0, 420.0]);
    assert!(rel(post.1, tube_area() * 400.0) < MEASURED, "{post:?} vs {}", tube_area() * 400.0);
}

#[test]
fn a_sketch_line_and_a_pair_member_butt_the_shorter_against_the_longer() {
    let mut s = Session::default();
    box_frame(&mut s);
    // The base's 600 bottom line and the 400 upright end together at (600, 0, 0).
    let bottom = line_at_y(&s, "Base", 0.0);
    let frame = |extra: Value| {
        let mut p = json!({"sketch": "Base", "lines": [bottom], "profile": tube(), "pairs": [upright()]});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let fid = run(&mut s, "frame.member", frame(json!({"corner": "butt"})))["feature"].as_u64().unwrap();
    let bs = members_measured(&mut s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    // The longer line runs through to the upright's outer face; the upright stops on its top.
    let line = member_at(&bs, [0.0, -20.0, -20.0], [620.0, 20.0, 20.0]);
    assert!(rel(line.1, tube_area() * 620.0) < MEASURED, "{line:?} vs {}", tube_area() * 620.0);
    let post = member_at(&bs, [580.0, -20.0, 20.0], [620.0, 20.0, 400.0]);
    assert!(rel(post.1, tube_area() * 380.0) < MEASURED, "{post:?} vs {}", tube_area() * 380.0);
    // Flipped, by a Mitre frame's override named by the line's and the pair's ids, the pair runs through.
    run(&mut s, "timeline.delete", json!({"feature": fid}));
    let over = json!({"corner": "mitre", "corners": [{"members": ["pair1", bottom], "corner": "butt", "flip": true}]});
    run(&mut s, "frame.member", frame(over));
    let bs = members_measured(&mut s);
    member_at(&bs, [0.0, -20.0, -20.0], [580.0, 20.0, 20.0]);
    member_at(&bs, [580.0, -20.0, -20.0], [620.0, 20.0, 400.0]);
}

#[test]
fn a_tie_runs_the_sketch_line_through_then_the_first_pair() {
    let mut s = Session::default();
    box_frame(&mut s);
    // The 400 left line (along Y) and a 400 post (along Z) at the origin: equally long, equally far
    // from the sketch's X axis. The sketch's line comes before every pair.
    let left = line_at_x(&s, "Base", 0.0);
    let post = json!({"a": {"vertex": [0, 0, 0]}, "b": {"vertex": [0, 0, 400]}});
    run(&mut s, "frame.member", json!({"sketch": "Base", "lines": [left], "profile": tube(), "pairs": [post], "corner": "butt"}));
    let bs = members_measured(&mut s);
    member_at(&bs, [-20.0, -20.0, -20.0], [20.0, 400.0, 20.0]);
    member_at(&bs, [-20.0, -20.0, 20.0], [20.0, 20.0, 400.0]);
    // Two pairs alike (the upright and the 400 top side at (600, 0, 400)): the first runs through.
    let side = json!({"a": {"vertex": [600, 0, 400]}, "b": {"vertex": [600, 400, 400]}});
    for (pairs, through) in [
        (json!([upright(), side]), ([580.0, -20.0, 0.0], [620.0, 20.0, 420.0])),
        (json!([side, upright()]), ([580.0, -20.0, 380.0], [620.0, 400.0, 420.0])),
    ] {
        let fid = run(&mut s, "frame.member", json!({"profile": tube(), "pairs": pairs, "corner": "butt"}))["feature"].as_u64().unwrap();
        let bs = feature_boxes(&mut s, fid);
        assert!(bs.values().any(|b| near3(b.0, through.0) && near3(b.1, through.1)), "{pairs}: {bs:?}");
        run(&mut s, "timeline.delete", json!({"feature": fid}));
    }
}

#[test]
fn removing_a_tube_drops_the_corner_overrides_that_name_it() {
    let mut s = Session::default();
    box_frame(&mut s);
    let bottom = line_at_y(&s, "Base", 0.0);
    let p = json!({"sketch": "Base", "lines": [bottom], "profile": tube(), "pairs": [upright(), top_rail()], "corner": "mitre",
                   "corners": [{"members": [bottom, "pair1"], "corner": "butt"}, {"members": ["pair1", "pair2"], "corner": "butt"}]});
    let fid = run(&mut s, "frame.member", p)["feature"].clone();
    remove(&mut s, json!([{"feature": fid, "line": "pair1"}]));
    let solidcraft_doc::FeatureKind::FrameMember { corners, .. } = member_kind(&s, &fid) else { panic!() };
    assert!(corners.is_empty(), "{corners:?}");
    assert_eq!(member_boxes(&mut s).len(), 2, "the other tubes build: the overrides no longer point at a corner that has gone");
}

// ---------------------------------------------------------------------------------------------
// Auto corners (#4003 stage 2): each joint by its angle, and the frame's own tubes cut to fit

/// A `polyline_frame` of a 400 line along +X to the origin and a 300 line out of it at `theta` degrees, "auto".
fn auto_vee(theta: f64) -> (Session, Result<Value, String>) {
    let t = theta.to_radians();
    polyline_frame(json!([[400, 0], [0, 0], [300.0 * t.cos(), 300.0 * t.sin()]]), "auto")
}

/// The long (400, reaching x > 300) and the short member of an `auto_vee`.
fn long_and_short(s: &mut Session) -> (Measured, Measured) {
    let bs = bodies(s);
    assert_eq!(bs.len(), 2, "{bs:?}");
    let long = bs.iter().find(|b| b.3[0] > 300.0).unwrap_or_else(|| panic!("{bs:?}")).clone();
    let short = bs.iter().find(|b| b.0 != long.0).unwrap().clone();
    (long, short)
}

#[test]
fn auto_at_20_degrees_is_an_angled_butt() {
    // The 300 leaves at 160°: 20° from running on in line with the 400 (along −X). The table's angled butt.
    let (mut s, r) = auto_vee(160.0);
    r.unwrap();
    let (long, short) = long_and_short(&mut s);
    // The long tube is cut square to its own line at the corner (x = 0), so its box runs x 0…400 and ±20
    // (the half-width) across: [0, −20, −20]…[400, 20, 20], and its volume stays A·400.
    assert!(near3(long.2, [0.0, -20.0, -20.0]) && near3(long.3, [400.0, 20.0, 20.0]), "{long:?}");
    assert!(rel(long.1, tube_area() * 400.0) < MEASURED, "{long:?} vs {}", tube_area() * 400.0);
    // The short tube is cut on that same plane x = 0, square to the long line, keeping x ≤ 0: it sits on the
    // long tube's square end, so its box ends at x = 0 (left square, it would reach 20·sin 20° = 6.84 past it).
    // The plane passes through its line at the corner, through the section's centroid, so its volume is A·300.
    assert!(short.3[0].abs() < 1e-6, "{short:?}");
    assert!(rel(short.1, tube_area() * 300.0) < MEASURED, "{short:?} vs {}", tube_area() * 300.0);
    // The 400 is drawn first, (400,0)→(0,0): the corner is its end and the 300's start.
    let st = s.model.state();
    for (b, role) in [(&long, "corner_end"), (&short, "corner_start")] {
        let names = solidcraft_doc::naming::face_names(st.body(&b.0).unwrap());
        assert!(names.iter().any(|n| n.contains(role)), "{role} in {names:?}");
    }
}

#[test]
fn auto_at_45_and_140_degrees_mitres() {
    for swing in [45.0f64, 140.0] {
        let (mut s, r) = auto_vee(180.0 - swing);
        r.unwrap_or_else(|e| panic!("{swing}: {e}"));
        let (long, short) = long_and_short(&mut s);
        // A mitre's plane passes through both lines at the corner: each keeps A × its line length.
        assert!(rel(long.1, tube_area() * 400.0) < MEASURED, "{swing}: {long:?} vs {}", tube_area() * 400.0);
        assert!(rel(short.1, tube_area() * 300.0) < MEASURED, "{swing}: {short:?} vs {}", tube_area() * 300.0);
        // The inside angle is 180 − swing; the 400 runs past the corner on its outer side (y = −20) to where the
        // bisecting plane meets it: x = −20 / tan(inside / 2): −8.284 at 45°, −54.950 at 140°.
        let reach = -20.0 / ((180.0 - swing).to_radians() / 2.0).tan();
        assert!((long.2[0] - reach).abs() < 1e-6, "{swing}: {reach} {long:?}");
    }
    // A swing of 176° (4° inside) is a mitre too sharp to finish, as stage 1 says.
    let (mut s, r) = auto_vee(4.0);
    let e = r.unwrap_err();
    assert!(e.contains("Corner at (0, 0, 0) is too sharp to finish"), "{e}");
    assert!(bodies(&mut s).is_empty());
}

#[test]
fn auto_at_70_90_110_degrees_butts_and_fits() {
    for swing in [70.0f64, 90.0, 110.0] {
        let (mut s, r) = auto_vee(180.0 - swing);
        r.unwrap_or_else(|e| panic!("{swing}: {e}"));
        let (long, short) = long_and_short(&mut s);
        // T, the 400, runs through to U's outer face: h / sin φ = 20 / sin φ past the corner along its line
        // (21.28 at 70° and 110°, 20 at 90°).
        let sn = swing.to_radians().sin();
        let t = tube_area() * (400.0 + 20.0 / sn);
        assert!(rel(long.1, t) < MEASURED, "{swing}: T {long:?} vs {t}");
        // U, the 300, is fitted to T's squared-off box: flat on T's near face, a plane 20 from T's line which meets
        // U's line 20 / sin φ from the corner. So U keeps A·(300 − 20 / sin φ).
        let u = tube_area() * (300.0 - 20.0 / sn);
        assert!(rel(short.1, u) < MEASURED, "{swing}: U {short:?} vs {u}");
    }
}

#[test]
fn auto_rectangle_runs_its_long_sides_through() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "auto"}));
    // As stage 1's Butt rectangle: the 600s run out to the 400s' outer faces (x −20…620, A·640), and the 400s
    // stop on the 600s' near faces (y 20…380, A·360), fitted flat to the squared box tube.
    let (bottom, left) = bottom_and_left(&mut s);
    assert!((bottom.2[0] + 20.0).abs() < 1e-6 && (bottom.3[0] - 620.0).abs() < 1e-6, "{bottom:?}");
    assert!(rel(bottom.1, tube_area() * 640.0) < MEASURED, "{bottom:?} vs {}", tube_area() * 640.0);
    assert!((left.2[1] - 20.0).abs() < 1e-6 && (left.3[1] - 380.0).abs() < 1e-6, "{left:?}");
    assert!(rel(left.1, tube_area() * 360.0) < MEASURED, "{left:?} vs {}", tube_area() * 360.0);
    let st = s.model.state();
    let names = solidcraft_doc::naming::face_names(st.body(&bottom.0).unwrap());
    for role in ["corner_start", "corner_end"] {
        assert!(names.iter().any(|n| n.contains(role)), "{role} in {names:?}");
    }
    // The left's fitted faces are named for the end they finish and the line they fit: (0,400)→(0,0) starts
    // against the top and ends against the bottom.
    let names = solidcraft_doc::naming::face_names(st.body(&left.0).unwrap());
    for (role, y) in [("corner_start", 400.0), ("corner_end", 0.0)] {
        let fits = format!("{role}:{}:face:", line_at_y(&s, "Frame", y));
        assert!(names.iter().any(|n| n.contains(&fits)), "{fits} in {names:?}");
    }
    // In round tube the 400s are fitted, not cut flat: a fishmouth whose tips reach the 600s' centrelines at
    // their sides (y 0…400), A·400 less, at each end, the part inside the 600: stage 1's coped value,
    // A·360 + 2∫(20 − √(400 − z²)) dA = 129458.17.
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let round = json!({"kind": "round_tube", "diameter": 40, "wall": 3});
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": round, "corner": "auto"}));
    let (_, left) = bottom_and_left(&mut s);
    assert!(left.2[1].abs() < 1e-3 && (left.3[1] - 400.0).abs() < 1e-3, "{left:?}");
    assert!(rel(left.1, 129_458.17) < MEASURED, "{left:?} vs 129458.17");
}

/// `rectangle_frame` with the profile sketch `Ring`: circles of radius 20 and 17 about its origin.
fn ring_frame(s: &mut Session) {
    rectangle_frame(s);
    run(s, "sketch.create", json!({"plane": "YZ", "name": "Ring"}));
    run(s, "sketch.circle.center", json!({"center": [0, 0], "radius": 20}));
    run(s, "sketch.circle.center", json!({"center": [0, 0], "radius": 17}));
    run(s, "sketch.finish", json!({}));
}

/// Mirror (or a half turn) puts a round section's seam on the frame's outer side, where each 600's flat end
/// touches the 400 fitted to it along the 400's outer side; SMLib's check refused the fitted 400 there ("Face Near
/// missing Seam"). Under Auto each section builds as the unmirrored, unturned one: a picked ring, a round tube and a
/// rectangular tube, with the values of `auto_rectangle_runs_its_long_sides_through`.
#[test]
fn auto_builds_a_mirrored_or_turned_section() {
    let ring = PI * (400.0 - 289.0);
    let picked = json!({"profile_sketch": "Ring", "profiles": [{"point": [18.5, 0]}]});
    let round = json!({"profile": {"kind": "round_tube", "diameter": 40, "wall": 3}});
    // (section, its area, the 400s' y range and volume): fishmouthed to the 600s' centrelines in a round section,
    // flat on their near faces in the rectangular tube.
    let sections = [
        (picked, ring, (0.0, 400.0), 129_458.17),
        (round, ring, (0.0, 400.0), 129_458.17),
        (json!({"profile": tube()}), tube_area(), (20.0, 380.0), tube_area() * 360.0),
    ];
    for (section, area, (y0, y1), short) in sections {
        for variant in [json!({"mirror": true}), json!({"angle": "180 deg"})] {
            let mut s = Session::default();
            ring_frame(&mut s);
            let mut p = json!({"sketch": "Frame", "corner": "auto"});
            for (k, v) in section.as_object().unwrap().iter().chain(variant.as_object().unwrap()) {
                p[k] = v.clone();
            }
            if let Err(e) = s.execute("frame.member", &p) {
                panic!("{p}: {e}");
            }
            assert_eq!(bodies(&mut s).len(), 4, "{p}");
            let (bottom, left) = bottom_and_left(&mut s);
            assert!(near3(bottom.2, [-20.0, -20.0, -20.0]) && near3(bottom.3, [620.0, 20.0, 20.0]), "{p}: {bottom:?}");
            assert!(rel(bottom.1, area * 640.0) < MEASURED, "{p}: {bottom:?} vs {}", area * 640.0);
            assert!((left.2[1] - y0).abs() < 1e-3 && (left.3[1] - y1).abs() < 1e-3, "{p}: {left:?}");
            assert!(rel(left.1, short) < MEASURED, "{p}: {left:?} vs {short}");
        }
    }
}

/// The New Frame Member dialog's picked ring, anchored at its bottom, turned 15° and mirrored (stage 1's dialog test
/// `a_picked_section_and_the_placement`), builds under Auto: a ring mirrored is the same ring, so each member is the
/// unmirrored one's, to the volume and the box.
#[test]
fn auto_builds_the_dialogs_mirrored_picked_ring() {
    let build = |mirror: bool| {
        let mut s = Session::default();
        ring_frame(&mut s);
        let p = json!({"sketch": "Frame", "profile_sketch": "Ring", "profiles": [{"point": [18.5, 0]}], "corner": "auto",
                       "anchor": "bottom", "angle": "15 deg", "mirror": mirror, "body_name": "Rail"});
        if let Err(e) = s.execute("frame.member", &p) {
            panic!("{p}: {e}");
        }
        bodies(&mut s)
    };
    let (mirrored, plain) = (build(true), build(false));
    let names: Vec<&str> = mirrored.iter().map(|b| b.0.as_str()).collect();
    assert_eq!(names, ["Rail", "Rail (2)", "Rail (3)", "Rail (4)"]);
    for (m, p) in mirrored.iter().zip(&plain) {
        assert!(m.0 == p.0 && rel(m.1, p.1) < MEASURED && near3(m.2, p.2) && near3(m.3, p.3), "{m:?} vs {p:?}");
    }
}

/// The 600 × 400 rectangle `Frame` with a cross-member from (300, 0) to (300, 400).
fn cross_member_frame(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(s, "sketch.line", json!({"points": [[300, 0], [300, 400]]}));
    run(s, "sketch.finish", json!({}));
}

#[test]
fn auto_fits_a_t_junction() {
    let mut s = Session::default();
    cross_member_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "auto"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 5, "{bs:?}");
    // The bottom and top pass through x = 300: the cross-member's ends are fitted flat to their near faces,
    // y = 20 and y = 380, leaving 360 of it.
    let cross = bs.iter().find(|b| b.2[0] > 250.0 && b.3[0] < 350.0).unwrap_or_else(|| panic!("{bs:?}"));
    assert!(near3(cross.2, [280.0, 20.0, -20.0]) && near3(cross.3, [320.0, 380.0, 20.0]), "{cross:?}");
    assert!(rel(cross.1, tube_area() * 360.0) < MEASURED, "{cross:?} vs {}", tube_area() * 360.0);
}

#[test]
fn auto_with_crossing_tubes_overlaps_without_error() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [600, 400]]}));
    run(&mut s, "sketch.line", json!({"points": [[0, 400], [600, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "auto"}));
    let bs = bodies(&mut s);
    assert_eq!(bs.len(), 6, "{bs:?}");
    // Each diagonal ends at two rectangle corners, where its end is fitted to the two sides' tubes (x ≤ 20 and
    // y ≤ 20 go). Its edge 20 above its line meets x = 20 at y = 37.4, the one below meets y = 20 at x = 66.0,
    // so it starts at x = 20 and y = 20, and ends likewise at the far corner: [20, 20, −20]…[580, 380, 20]. It
    // runs on across the other diagonal whole: one body each, the crossing left overlapping.
    let diagonals = bs.iter().filter(|b| near3(b.2, [20.0, 20.0, -20.0]) && near3(b.3, [580.0, 380.0, 20.0])).count();
    assert_eq!(diagonals, 2, "{bs:?}");
}

#[test]
fn auto_overrides_at_three_ends() {
    // a (600) along X, b (400) down Y to the origin, c (283) out at 45°: a and b, laid first, make the corner
    // (a butt at 90°), and c is fitted to both.
    let build = |flip: Option<bool>| {
        let mut s = Session::default();
        run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Y"}));
        run(&mut s, "sketch.line", json!({"points": [[0, 0], [600, 0]], "ids": ["a"]}));
        run(&mut s, "sketch.line", json!({"points": [[0, 400], [0, 0]], "ids": ["b"]}));
        run(&mut s, "sketch.line", json!({"points": [[0, 0], [200, 200]], "ids": ["c"]}));
        run(&mut s, "sketch.finish", json!({}));
        let mut p = json!({"sketch": "Y", "profile": tube(), "corner": "auto", "names": {"a": "a", "b": "b", "c": "c"}});
        if let Some(flip) = flip {
            p["corners"] = json!([{"members": ["c", "a", "b"], "corner": "auto", "flip": flip}]);
        }
        run(&mut s, "frame.member", p);
        boxes(&mut s)
    };
    // c: fitted to x ≤ 20 and y ≤ 20 near the origin (its edges meet them 48.3 along it); its far end is square
    // at (200, 200), its corners ±20·(−0.707, 0.707) off it: x and y up to 214.142.
    let h = 20.0 * std::f64::consts::FRAC_1_SQRT_2;
    let c = ([20.0, 20.0, -20.0], [200.0 + h, 200.0 + h, 20.0]);
    for flip in [None, Some(false)] {
        let bs = build(flip);
        // Unflipped, the longer a runs through to b's outer face (x = −20); b is fitted to a's near face (y = 20).
        assert!(near3(bs["a"].0, [-20.0, -20.0, -20.0]) && near3(bs["a"].1, [600.0, 20.0, 20.0]), "{flip:?} {bs:?}");
        assert!(near3(bs["b"].0, [-20.0, 20.0, -20.0]) && near3(bs["b"].1, [20.0, 400.0, 20.0]), "{flip:?} {bs:?}");
        assert!(near3(bs["c"].0, c.0) && near3(bs["c"].1, c.1), "{flip:?} {bs:?}");
    }
    // Flipped, the roles swap: b runs through to a's outer face (y = −20), a is fitted to b's near face (x = 20).
    let bs = build(Some(true));
    assert!(near3(bs["b"].0, [-20.0, -20.0, -20.0]) && near3(bs["b"].1, [20.0, 400.0, 20.0]), "{bs:?}");
    assert!(near3(bs["a"].0, [20.0, -20.0, -20.0]) && near3(bs["a"].1, [600.0, 20.0, 20.0]), "{bs:?}");
    assert!(near3(bs["c"].0, c.0) && near3(bs["c"].1, c.1), "{bs:?}");
}

/// Each body's volume and box, and its sorted face names, by body name.
type Built = (std::collections::BTreeMap<String, (f64, [f64; 3], [f64; 3])>, std::collections::BTreeMap<String, Vec<String>>);

fn built(s: &mut Session) -> Built {
    let faces = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| {
            let mut n = solidcraft_doc::naming::face_names(b).to_vec();
            n.sort();
            (b.name.clone(), n)
        })
        .collect();
    (bodies(s).into_iter().map(|b| (b.0, (b.1, b.2, b.3))).collect(), faces)
}

#[test]
fn auto_is_the_same_whatever_the_pick_order() {
    let build = |reverse: bool| {
        let mut s = Session::default();
        cross_member_frame(&mut s);
        let mut lines: Vec<String> = sketch_lines(&s, "Frame").into_iter().map(|l| l.0).collect();
        assert_eq!(lines.len(), 5);
        if reverse {
            lines.reverse();
        }
        let names: serde_json::Map<String, Value> = lines.iter().map(|l| (l.clone(), json!(l))).collect();
        run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "auto", "lines": lines, "names": names}));
        built(&mut s)
    };
    let ((drawn, drawn_faces), (picked, picked_faces)) = (build(false), build(true));
    assert_eq!(drawn.keys().collect::<Vec<_>>(), picked.keys().collect::<Vec<_>>());
    for (line, (v, lo, hi)) in &drawn {
        let (pv, plo, phi) = picked[line];
        assert!(rel(pv, *v) < 1e-9 && near3(plo, *lo) && near3(phi, *hi), "{line}: drawn {drawn:?} picked {picked:?}");
    }
    assert_eq!(drawn_faces, picked_faces);
}

#[test]
fn a_non_auto_frame_is_unchanged() {
    let mut s = Session::default();
    rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre"}));
    assert_stage_one_mitre_rectangle(&mut s);
}

/// Stage 1's Mitre rectangle, exactly. Recorded on `frame-auto-joints` at 4afc6a0c2, before Auto was built: there the
/// non-Auto path was `frame-corner-cuts`' (3681bf74a) unchanged (`git diff 3681bf74a 4afc6a0c2` changes only override
/// parsing for 3+ members). Each member runs out to the rectangle's outer corners, 20 past its line's ends, and keeps
/// A × its line: A·600 = 252493.80, A·400 = 168329.20.
fn assert_stage_one_mitre_rectangle(s: &mut Session) {
    let (got, faces) = built(s);
    let want = [
        ("Rect Tube 40x40x3", 252_493.801_996_584_55, [-20.0, -20.0, -20.0], [620.0, 20.0, 20.0]),
        ("Rect Tube 40x40x3 (2)", 168_329.201_030_185_65, [580.0, -20.0, -20.0], [620.0, 420.0, 20.0]),
        ("Rect Tube 40x40x3 (3)", 252_493.801_996_583_7, [-20.0, 380.0, -20.0], [620.0, 420.0, 20.0]),
        ("Rect Tube 40x40x3 (4)", 168_329.201_030_186_87, [-20.0, -20.0, -20.0], [20.0, 420.0, 20.0]),
    ];
    assert_eq!(got.len(), 4, "{got:?}");
    // Its two mitred ends named for the ends they finish, and the tube's own faces.
    let mut names: Vec<String> = ["corner_end", "corner_start"].iter().map(|n| format!("F2:{n}")).collect();
    names.extend([0, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17].iter().map(|k| format!("F2:face:{k}")));
    names.sort();
    for (name, v, lo, hi) in want {
        let (gv, glo, ghi) = got[name];
        assert!(rel(gv, v) < 1e-12 && near3(glo, lo) && near3(ghi, hi), "{name}: {:?}", got[name]);
        assert_eq!(faces[name], names, "{name}");
    }
}

#[test]
fn an_auto_override_on_a_mitre_frame_builds_as_mitre() {
    // Only an Auto frame builds by Auto's rules: on a Mitre frame an Auto override is the frame's own Mitre.
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let (bottom, left) = (line_at_y(&s, "Frame", 0.0), line_at_x(&s, "Frame", 0.0));
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Frame", "profile": tube(), "corner": "mitre", "corners": [{"members": [bottom, left], "corner": "auto"}]}),
    );
    assert_stage_one_mitre_rectangle(&mut s);
}

#[test]
fn an_auto_override_on_a_butt_frame_keeps_its_flip() {
    // On a Butt frame, an Auto override with Flip at the origin is a flipped Butt there, built as stage 1 builds a
    // Butt override with Flip.
    let build = |kind: &str| {
        let mut s = Session::default();
        rectangle_frame(&mut s);
        let (bottom, left) = (line_at_y(&s, "Frame", 0.0), line_at_x(&s, "Frame", 0.0));
        run(
            &mut s,
            "frame.member",
            json!({"sketch": "Frame", "profile": tube(), "corner": "butt",
                   "corners": [{"members": [bottom, left], "corner": kind, "flip": true}]}),
        );
        let (b, l) = bottom_and_left(&mut s);
        (b, l, built(&mut s))
    };
    let (bottom, left, auto) = build("auto");
    // Flipped at the origin: the left runs through to the bottom's outer face (y = −20), the bottom stops on the
    // left's near face (x = 20). The other corners are the frame's unflipped Butt: the bottom runs out to x = 620,
    // the left stops on the top's near face, y = 380.
    assert!((left.2[1] + 20.0).abs() < 1e-6 && (left.3[1] - 380.0).abs() < 1e-6, "{left:?}");
    assert!((bottom.2[0] - 20.0).abs() < 1e-6 && (bottom.3[0] - 620.0).abs() < 1e-6, "{bottom:?}");
    // And exactly as stage 1's Butt override with Flip: volumes, boxes and face names.
    let (_, _, butt) = build("butt");
    assert_eq!(auto.1, butt.1);
    for (name, (v, lo, hi)) in &butt.0 {
        let (av, alo, ahi) = auto.0[name];
        assert!(rel(av, *v) < 1e-12 && near3(alo, *lo) && near3(ahi, *hi), "{name}: auto {:?} butt {:?}", auto.0, butt.0);
    }
}

#[test]
fn a_three_tube_override_on_a_mitre_frame_still_fails() {
    // A hand-edited design: a Mitre frame whose two-member Auto override is made three members. An Auto override
    // doesn't make the frame Auto, so it is not at a corner of the frame (three tubes never meet at one).
    let mut s = Session::default();
    rectangle_frame(&mut s);
    let l = sketch_lines(&s, "Frame");
    let (a, b, c) = (&l[0].0, &l[1].0, &l[2].0);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre", "corners": [{"members": [a, b], "corner": "auto"}]}));
    let member = s.doc.features.iter().find(|f| matches!(f.kind, solidcraft_doc::FeatureKind::FrameMember { .. })).unwrap().id;
    let json = serde_json::from_str::<Value>(&s.doc.to_json()).unwrap().to_string();
    let from = format!("\"members\":[\"{a}\",\"{b}\"]");
    assert!(json.contains(&from), "{json}");
    let edited = json.replace(&from, &format!("\"members\":[\"{a}\",\"{b}\",\"{c}\"]"));
    let reopened = Session::new(solidcraft_doc::Document::from_json(&edited).unwrap());
    let e = reopened.model.result(member).and_then(|r| r.error.clone()).unwrap_or_default();
    assert!(e.contains("A corner override is not at a corner of the frame"), "{e}");
}

// ---------------------------------------------------------------------------------------------
// Auto lines sections up (#4003 stage 2, Goal 7): a group's sketch normal is up, outer tubes run round the outline

/// A 600 × 400 rectangle on XZ (the front plane: sketch x = +X, sketch y = −Z, normal +Y), as the frame sketch
/// `Frame`: x 0…600, z 0…400, drawn anticlockwise in the sketch, so Auto runs every side the way it is drawn.
fn vertical_rectangle_frame(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XZ", "name": "Frame"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, -400]}));
    run(s, "sketch.finish", json!({}));
}

/// A 40 × 20 flat bar, square cornered: A = 800.
fn flat_bar() -> Value {
    json!({"kind": "bar", "width": 40, "height": 20})
}

/// Every body's (min, max, volume), sorted by box.
fn sorted_boxes(s: &mut Session) -> Vec<([f64; 3], [f64; 3], f64)> {
    let mut bs: Vec<_> = bodies(s).into_iter().map(|b| (b.2, b.3, b.1)).collect();
    // By the box to 1e-3, so rounding never reorders two boxes that share a corner coordinate.
    bs.sort_by_key(|b| b.0.iter().chain(&b.1).map(|v| (v * 1e3).round() as i64).collect::<Vec<_>>());
    bs
}

/// `got` are `want` (min, max, volume), boxes to 1e-6 and volumes to MEASURED.
fn assert_boxes(got: &[([f64; 3], [f64; 3], f64)], want: &[([f64; 3], [f64; 3], f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(near3(g.0, w.0) && near3(g.1, w.1) && rel(g.2, w.2) < MEASURED, "got {g:?} want {w:?}\nall {got:?}");
    }
}

#[test]
fn auto_lines_up_a_vertical_rectangle() {
    let mut s = Session::default();
    vertical_rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": flat_bar(), "corner": "auto"}));
    // Up is the sketch's normal, +Y, for all four: each bar's height (20) lies across the frame's plane, y −10…10,
    // and its width (40) in the plane, ±20 about its line. (By world Z the 600s would lie flat, y −20…20.)
    // Butts at 90°: the 600s run through to the 400s' outer faces, 20 past each end, x −20…620: A·640 = 512000.
    // The 400s are fitted flat to the 600s' near faces, z 20…380: A·360 = 288000.
    let (long, short) = (800.0 * 640.0, 800.0 * 360.0);
    assert_boxes(
        &sorted_boxes(&mut s),
        &[
            ([-20.0, -10.0, -20.0], [620.0, 10.0, 20.0], long),
            ([-20.0, -10.0, 20.0], [20.0, 10.0, 380.0], short),
            ([-20.0, -10.0, 380.0], [620.0, 10.0, 420.0], long),
            ([580.0, -10.0, 20.0], [620.0, 10.0, 380.0], short),
        ],
    );
}

#[test]
fn auto_lines_up_head_to_head_lines() {
    // Two lines drawn away from the corner at the origin on XZ: along −X (600) and along +Z (400, sketch y −400).
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "L"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [-600, 0]]}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [0, -400]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "L", "profile": flat_bar(), "corner": "auto", "anchor": "bottom_left"}));
    // Up is +Y for both, and the bottom-left anchor (−20, −10) puts each section's y 10 above its line: both sit on
    // the +Y side of the frame's plane, y 0…20. (By world Z, the −X line's x would be −Y and its bar y −40…0.)
    // In the plane each section's x is y × its run: Z along −X, X along +Z, so the 600 occupies z 0…40 and the 400
    // x 0…40. The 600 runs through to the 400's far face, x 40 (A·640); the 400 is fitted to the 600's top, z 40
    // (A·360).
    assert_boxes(
        &sorted_boxes(&mut s),
        &[([-600.0, 0.0, 0.0], [40.0, 20.0, 40.0], 800.0 * 640.0), ([0.0, 0.0, 40.0], [40.0, 20.0, 400.0], 800.0 * 360.0)],
    );
}

/// A 600 × 400 rectangle on XY drawn clockwise seen from +Z, as the frame sketch `Frame`, with Auto members of
/// `profile` anchored bottom left. Each side runs against the anticlockwise outline, so Auto turns its section round:
/// then x = up × run points into the rectangle for every side, and the bottom-left anchor (−20, −20) puts each 40
/// tube wholly inside it, 0…40 in from its line and z 0…40.
fn clockwise_auto_frame(profile: Value) -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [0, 400], [600, 400], [600, 0], [0, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": profile, "corner": "auto", "anchor": "bottom_left"}));
    s
}

#[test]
fn auto_runs_outer_tubes_round_the_outline() {
    let mut s = clockwise_auto_frame(tube());
    // The 600s run through to the 400s' outer faces, which lie on the lines x = 0 and 600: A·600. The 400s are
    // fitted to the 600s' inner faces, y 40 and 360: A·320.
    let (long, short) = (tube_area() * 600.0, tube_area() * 320.0);
    assert_boxes(
        &sorted_boxes(&mut s),
        &[
            ([0.0, 0.0, 0.0], [600.0, 40.0, 40.0], long),
            ([0.0, 40.0, 0.0], [40.0, 360.0, 40.0], short),
            ([0.0, 360.0, 0.0], [600.0, 400.0, 40.0], long),
            ([560.0, 40.0, 0.0], [600.0, 360.0, 40.0], short),
        ],
    );
}

#[test]
fn auto_lines_up_a_3d_chain_in_its_plane() {
    // A post and a rail of a 3D sketch, in the plane y = 0: their chain's normal pointing up is +Y (no Z part).
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Tower"}));
    run(&mut s, "sketch.options", json!({"sketch_3d": true}));
    run(&mut s, "sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 1000], [600, 0, 1000]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Tower", "profile": {"kind": "bar", "width": 60, "height": 10}, "corner": "auto"}));
    // Both bars' height (10) along Y, y −5…5, their width (60) in the plane: the post's along X, the rail's along Z.
    // (By world Z the rail would lie flat, y −30…30.) Their chained corner is mitred on the plane through
    // (0, 0, 1000) square to (1, 0, 1): the post reaches z 1030 at x −30, the rail x −30 at z 1030. A mitre through
    // the centreline keeps A × the line: 600·1000 and 600·600.
    assert_boxes(
        &sorted_boxes(&mut s),
        &[([-30.0, -5.0, 0.0], [30.0, 5.0, 1030.0], 600_000.0), ([-30.0, -5.0, 970.0], [600.0, 5.0, 1030.0], 360_000.0)],
    );
}

#[test]
fn an_end_cap_sits_on_a_tube_auto_runs_against_its_line() {
    // The bottom, drawn (600, 0) → (0, 0), runs through and lies inside the rectangle, y 0…40, z 0…40; its ends are
    // cut square at x = 600 (its drawn start) and x = 0 (its drawn end), the 400s' outer faces. End Cap measures
    // the member from its drawn start, so both are open ends, and each cap covers the same 40 × 40, 6 thick past
    // its end: x 600…606 and −6…0.
    let mut s = clockwise_auto_frame(square_tube());
    let bottom = bodies(&mut s).into_iter().find(|b| b.3[0] - b.2[0] > 500.0 && b.3[1] < 100.0).unwrap();
    let start = face_pick(&s, &bottom.0, [600.0, 1.5, 20.0], [1.0, 0.0, 0.0]);
    let end = face_pick(&s, &bottom.0, [0.0, 1.5, 20.0], [-1.0, 0.0, 0.0]);
    run(&mut s, "frame.end_cap", json!({"faces": [start, end], "thickness": 6}));
    let mut caps: Vec<Measured> = bodies(&mut s).into_iter().filter(|b| b.0.contains("End Cap")).collect();
    caps.sort_by(|a, b| a.2[0].total_cmp(&b.2[0]));
    assert_eq!(caps.len(), 2, "{caps:?}");
    assert!(near3(caps[0].2, [-6.0, 0.0, 0.0]) && near3(caps[0].3, [0.0, 40.0, 40.0]), "{caps:?}");
    assert!(near3(caps[1].2, [600.0, 0.0, 0.0]) && near3(caps[1].3, [606.0, 40.0, 40.0]), "{caps:?}");
}

/// A face of a body, as a point on its plane and its outward normal.
type Pick = ([f64; 3], [f64; 3]);

/// An open U on XY: the bottom (0,0)→(600,0), the right drawn down (600,400)→(600,0), the top (600,400)→(0,400), in
/// Auto members of `member` (frame.member's profile and anchor), after `setup`. Open, nothing is outer and the right
/// runs as drawn. Closed by the left (0,400)→(0,0), the outline runs up the right side, so Auto runs the right against
/// its line. Gives the right tube and the names of its faces at `open`, then once the U is closed, at `closed`.
fn names_across_closing(setup: impl Fn(&mut Session), member: Value, open: &[Pick], closed: &[Pick]) -> [(Measured, Vec<String>); 2] {
    let mut s = Session::default();
    setup(&mut s);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    for points in [[[0, 0], [600, 0]], [[600, 400], [600, 0]], [[600, 400], [0, 400]]] {
        run(&mut s, "sketch.line", json!({"points": points}));
    }
    run(&mut s, "sketch.finish", json!({}));
    let mut p = json!({"sketch": "Frame", "corner": "auto"});
    for (k, v) in member.as_object().unwrap() {
        p[k] = v.clone();
    }
    run(&mut s, "frame.member", p);
    let named = |s: &mut Session, picks: &[Pick]| {
        let right = bodies(s).into_iter().find(|b| b.3[1] - b.2[1] > 300.0 && b.2[0] > 500.0).unwrap();
        let names = solidcraft_doc::naming::face_names(s.model.state().body(&right.0).unwrap()).to_vec();
        let picked = picks.iter().map(|(at, n)| names[face_pick(s, &right.0, *at, *n)["index"].as_u64().unwrap() as usize].clone()).collect();
        (right, picked)
    };
    let before = named(&mut s, open);
    run(&mut s, "sketch.edit", json!({"sketch": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 400], [0, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    assert_eq!(bodies(&mut s).len(), 4);
    [before, named(&mut s, closed)]
}

/// The walls of a 40 × 40 box tube with 3 mm walls whose box starts at x0 and z0: facing +X, −X, +Z, −Z outside, then
/// the bore's walls behind them.
fn box_walls(x0: f64, z0: f64) -> [Pick; 8] {
    let (mx, mz) = (x0 + 20.0, z0 + 20.0);
    [
        ([x0 + 40.0, 200.0, mz], [1.0, 0.0, 0.0]),
        ([x0, 200.0, mz], [-1.0, 0.0, 0.0]),
        ([mx, 200.0, z0 + 40.0], [0.0, 0.0, 1.0]),
        ([mx, 200.0, z0], [0.0, 0.0, -1.0]),
        ([x0 + 37.0, 200.0, mz], [-1.0, 0.0, 0.0]),
        ([x0 + 3.0, 200.0, mz], [1.0, 0.0, 0.0]),
        ([mx, 200.0, z0 + 37.0], [0.0, 0.0, -1.0]),
        ([mx, 200.0, z0 + 3.0], [0.0, 0.0, 1.0]),
    ]
}

/// The same tube before and after: the same box and volume.
fn assert_same_solid(before: &Measured, after: &Measured) {
    assert!(near3(before.2, after.2) && near3(before.3, after.3) && rel(after.1, before.1) < 1e-9, "{before:?} {after:?}");
}

#[test]
fn closing_an_outline_keeps_a_tubes_face_names() {
    // Each wall of the right tube keeps its name: the wall at the same place in its section, outer wall, bore wall,
    // top and bottom alike. Centred, the tube is the same solid either way round, x 580…620, z −20…20.
    let walls = box_walls(580.0, -20.0);
    let [(open, before), (closed, after)] = names_across_closing(|_| {}, json!({"profile": tube(), "anchor": "center"}), &walls, &walls);
    assert_same_solid(&open, &closed);
    assert_eq!(before, after);
    // Anchored bottom left (−20, −20), its section's x is up × run: +X running as drawn (−Y), the tube outside the U at
    // x 600…640; −X running up (+Y), inside the closed outline at x 560…600; z 0…40 both ways. Built from its section
    // mirrored, it is the open tube moved 40 along −X: each wall keeps the name of the wall it was moved from.
    let [(open, before), (closed, after)] =
        names_across_closing(|_| {}, json!({"profile": tube(), "anchor": "bottom_left"}), &box_walls(600.0, 0.0), &box_walls(560.0, 0.0));
    let at = |b: &Measured| [b.2[0], b.3[0], b.2[2], b.3[2]];
    assert!(at(&open) == [600.0, 640.0, 0.0, 40.0] && at(&closed) == [560.0, 600.0, 0.0, 40.0], "{open:?} {closed:?}");
    assert_eq!(before, after);
}

#[test]
fn a_sketch_section_off_its_origin_keeps_its_face_names() {
    // A square tube drawn in its own sketch about (100, 50), not about the sketch's origin: 40 × 40 with 3 mm walls
    // and square corners, placed by its centre. It is the same solid either way round, x 580…620, z −20…20, and each
    // wall keeps its name.
    let section = |s: &mut Session| {
        run(s, "sketch.create", json!({"plane": "XY", "name": "Tube section"}));
        run(s, "sketch.rectangle.two_point", json!({"p0": [80, 30], "p1": [120, 70]}));
        run(s, "sketch.rectangle.two_point", json!({"p0": [83, 33], "p1": [117, 67]}));
        run(s, "sketch.finish", json!({}));
    };
    let walls = box_walls(580.0, -20.0);
    let [(open, before), (closed, after)] =
        names_across_closing(section, json!({"profile_sketch": "Tube section", "anchor": "center"}), &walls, &walls);
    assert_same_solid(&open, &closed);
    assert_eq!(before, after);
}

#[test]
fn an_angle_section_keeps_its_face_names() {
    // A 60 × 40 × 5 angle, centred: its 60 leg along the section's x at its bottom (z −20…−15), its 40 leg up its
    // left side. Open, the right runs −Y and the section's x is +X: the 40 leg at x 570…575. Run the other way the
    // section is mirrored and the 40 leg is at x 625…630. Each face keeps the name of the face swept from the same
    // edge of the section: the 60 leg's outer (z −20) and inner (z −15) faces, and the 40 leg's outer face (x 570
    // facing −X, then x 630 facing +X) and its inner face (x 575, then 625).
    let open = [
        ([600.0, 200.0, -20.0], [0.0, 0.0, -1.0]),
        ([600.0, 200.0, -15.0], [0.0, 0.0, 1.0]),
        ([570.0, 200.0, 0.0], [-1.0, 0.0, 0.0]),
        ([575.0, 200.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let closed = [
        ([600.0, 200.0, -20.0], [0.0, 0.0, -1.0]),
        ([600.0, 200.0, -15.0], [0.0, 0.0, 1.0]),
        ([630.0, 200.0, 0.0], [1.0, 0.0, 0.0]),
        ([625.0, 200.0, 0.0], [-1.0, 0.0, 0.0]),
    ];
    let angle = json!({"kind": "angle", "width": 60, "height": 40, "thickness": 5});
    let [(_, before), (_, after)] = names_across_closing(|_| {}, json!({"profile": angle, "anchor": "center"}), &open, &closed);
    assert_eq!(before, after);
}

#[test]
fn mitre_frames_keep_todays_placement() {
    // Only Auto lines sections up: the vertical rectangle under Mitre places each bar as stage 1 does, by world Z.
    let mut s = Session::default();
    vertical_rectangle_frame(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": flat_bar(), "corner": "mitre"}));
    // The 600s lie flat: width along Y (y −20…20), height in the plane (z ±10). The 400s stand on edge: width along X
    // (x ±20), height along Y (y −10…10). Each mitre plane bisects its corner, through the corner point, so the
    // 600s' outer faces (z −10, 410) reach x −10 and 610, and the 400s' (x −20, 620) reach z −20 and 420. A plane
    // through the centreline keeps A × the line: A·600 = 480000, A·400 = 320000.
    assert_boxes(
        &sorted_boxes(&mut s),
        &[
            ([-20.0, -10.0, -20.0], [20.0, 10.0, 420.0], 320_000.0),
            ([-10.0, -20.0, -10.0], [610.0, 20.0, 10.0], 480_000.0),
            ([-10.0, -20.0, 390.0], [610.0, 20.0, 410.0], 480_000.0),
            ([580.0, -10.0, -20.0], [620.0, 10.0, 420.0], 320_000.0),
        ],
    );
}

// ---------------------------------------------------------------------------------------------
// Fitting to earlier Frame Members and their copies (#4003 Goal 6)

/// Frame Member `A`: the 600 × 400 rectangle `Frame` in 40 × 40 tube, its bodies `A`, `A (2)`…. Corner None
/// leaves each tube square, from its line's start to its end: the bottom is x 0…600, y −20…20, z −20…20, the top
/// y 380…420.
fn earlier_frame(s: &mut Session) {
    rectangle_frame(s);
    run(s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "name": "A", "body_name": "A"}));
}

/// A `size` square tube, 2 thick.
fn square_of(size: f64) -> Value {
    json!({"kind": "rect_tube", "width": size, "height": size, "wall": 2})
}

/// Frame Member `B` (bodies `B`, `B (2)`…) under Auto: `lines` in a sketch on XY raised `z`, in `size` square tube.
fn later_frame(s: &mut Session, z: f64, lines: &[[[f64; 2]; 2]], size: f64, fit_ends: bool) {
    let mut sketch = json!({"plane": "XY", "name": "Later"});
    if z != 0.0 {
        sketch["offset"] = json!(z);
    }
    run(s, "sketch.create", sketch);
    for l in lines {
        run(s, "sketch.line", json!({"points": l}));
    }
    run(s, "sketch.finish", json!({}));
    run(s, "frame.member", json!({"sketch": "Later", "profile": square_of(size), "corner": "auto", "fit_ends": fit_ends, "body_name": "B"}));
}

/// The bodies whose names start with `prefix`.
fn named(s: &mut Session, prefix: &str) -> Vec<Measured> {
    bodies(s).into_iter().filter(|b| b.0.starts_with(prefix)).collect()
}

/// The one body of B, as B's line (300, 0)…(300, 400) at `z` builds it: its bbox y runs `lo`…`hi`.
fn assert_b_runs(s: &mut Session, lo: f64, hi: f64) {
    let bs = named(s, "B");
    assert_eq!(bs.len(), 1, "{bs:?}");
    let b = &bs[0];
    assert!((b.2[1] - lo).abs() < 1e-6 && (b.3[1] - hi).abs() < 1e-6, "B runs y {}…{}, not {lo}…{hi}: {b:?}", b.2[1], b.3[1]);
}

#[test]
fn fit_ends_fits_a_later_frame_to_an_earlier_one() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    later_frame(&mut s, 0.0, &[[[300.0, 0.0], [300.0, 400.0]]], 25.0, true);
    // B's ends lie on A's bottom tube (y −20…20) and top tube (y 380…420): each is fitted flat to the tube's near
    // face, so B runs y 20…380.
    assert_b_runs(&mut s, 20.0, 380.0);
}

#[test]
fn without_fit_ends_the_later_frame_overlaps() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    later_frame(&mut s, 0.0, &[[[300.0, 0.0], [300.0, 400.0]]], 25.0, false);
    // Square line ends: y 0…400, overlapping A's tubes.
    assert_b_runs(&mut s, 0.0, 400.0);
}

#[test]
fn two_braces_meeting_inside_an_earlier_tube_are_fitted_not_mitred() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    // Two braces from (300, 0), inside A's bottom tube, out to (150, 200) and (450, 200): a corner of B at 106°
    // that Auto would butt, but it lies inside an earlier tube, so it isn't jointed. Both ends are fitted flat to
    // the tube's near face, y = 20; jointed, the through brace would run on to its corner cut, below y = 0.
    later_frame(&mut s, 0.0, &[[[300.0, 0.0], [150.0, 200.0]], [[300.0, 0.0], [450.0, 200.0]]], 20.0, true);
    let bs = named(&mut s, "B");
    assert_eq!(bs.len(), 2, "{bs:?}");
    for b in &bs {
        assert!((b.2[1] - 20.0).abs() < 1e-6, "{b:?}");
    }
}

#[test]
fn a_moved_earlier_frame_is_fitted_where_it_is() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    let a: Vec<String> = named(&mut s, "A").into_iter().map(|b| b.0).collect();
    run(&mut s, "solid.move", json!({"bodies": a, "translate": [0, 0, 100]}));
    // A's tubes now lie at z 80…120: B, 100 up, ends on them and is fitted to them, y 20…380.
    later_frame(&mut s, 100.0, &[[[300.0, 0.0], [300.0, 400.0]]], 25.0, true);
    assert_b_runs(&mut s, 20.0, 380.0);
}

#[test]
fn a_mirrored_earlier_frame_is_fitted() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    let a: Vec<String> = named(&mut s, "A").into_iter().map(|b| b.0).collect();
    run(&mut s, "solid.mirror", json!({"bodies": a, "plane": "YZ"}));
    // The copies lie at x −600…0: B at x = −300 ends on the mirrored bottom and top tubes and is fitted to their
    // faces, y 20…380 (A's own tubes are 300 away).
    later_frame(&mut s, 0.0, &[[[-300.0, 0.0], [-300.0, 400.0]]], 25.0, true);
    assert_b_runs(&mut s, 20.0, 380.0);
}

#[test]
fn a_patterned_earlier_frame_is_fitted() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    run(&mut s, "solid.pattern.rectangular", json!({"features": ["A"], "dir1": [0, 0, 1], "count1": 2, "spacing1": 300}));
    // The second instance's tubes lie at z 280…320: B, 300 up, is fitted to them, y 20…380.
    later_frame(&mut s, 300.0, &[[[300.0, 0.0], [300.0, 400.0]]], 25.0, true);
    assert_b_runs(&mut s, 20.0, 380.0);
}

#[test]
fn an_end_on_the_second_diagonal_of_an_earlier_x_brace_is_fitted_to_it() {
    // A: an X brace of solid 40 × 40 bar centred on its lines, (0, 0)…(400, 400) then (0, 400)…(400, 0), crossing at
    // (200, 200), where each diagonal's middle lies inside the other. B's line runs square off the second diagonal
    // from (300, 100) on it: its end is fitted to that diagonal's side, so B and A (2) don't overlap. The same for
    // A's copies mirrored across YZ, which B meets at (−300, 100): B overlaps no tube of A or of its copies.
    for mirrored in [false, true] {
        let mut s = Session::default();
        run(&mut s, "sketch.create", json!({"plane": "XY", "name": "X"}));
        run(&mut s, "sketch.line", json!({"points": [[0, 0], [400, 400]]}));
        run(&mut s, "sketch.line", json!({"points": [[0, 400], [400, 0]]}));
        run(&mut s, "sketch.finish", json!({}));
        let bar = json!({"kind": "bar", "width": 40, "height": 40});
        run(&mut s, "frame.member", json!({"sketch": "X", "profile": bar, "anchor": "center", "name": "A", "body_name": "A"}));
        let a: Vec<String> = named(&mut s, "A").into_iter().map(|b| b.0).collect();
        if mirrored {
            run(&mut s, "solid.mirror", json!({"bodies": a, "plane": "YZ"}));
        }
        let x = if mirrored { -1.0 } else { 1.0 };
        later_frame(&mut s, 0.0, &[[[300.0 * x, 100.0], [450.0 * x, 250.0]]], 25.0, true);
        for other in bodies(&mut s).into_iter().map(|b| b.0).filter(|n| n != "B") {
            let i = run(&mut s, "inspect.interference", json!({"bodies": ["B", other]}));
            let o: f64 = i["interferences"].as_array().map_or(0.0, |all| all.iter().map(|x| f(&x["volume_mm3"])).sum());
            assert!(o < 1e-3, "mirrored {mirrored}: B overlaps {other} by {o} mm³");
        }
    }
}

#[test]
fn earlier_tubes_are_never_changed() {
    let mut s = Session::default();
    earlier_frame(&mut s);
    let before = named(&mut s, "A");
    later_frame(&mut s, 0.0, &[[[300.0, 0.0], [300.0, 400.0]], [[0.0, 200.0], [600.0, 200.0]]], 25.0, true);
    assert_eq!(named(&mut s, "A"), before);
}

#[test]
fn a_later_group_in_another_plane_is_fitted_where_it_shares_a_point() {
    // A: a side frame, the 600 × 400 rectangle on XZ under Auto, a corner of it at the origin. B: on XY, an L from
    // the origin out to (0, −400) and on to (600, −400). B's end at the origin lies in A's corner: it is fitted to
    // A's tubes there, whose faces lie at y = ±20 (A is laid in XZ), so B's first tube runs y −400…−20 (from its
    // line's far end, where B's own corner is jointed; its 40 × 40 section reaches y −420 round that corner).
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "Side"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Side", "profile": tube(), "corner": "auto", "name": "A", "body_name": "A"}));
    let before = named(&mut s, "A");
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Floor"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [0, -400]]}));
    run(&mut s, "sketch.line", json!({"points": [[0, -400], [600, -400]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Floor", "profile": tube(), "corner": "auto", "fit_ends": true, "body_name": "B"}));
    let bs = named(&mut s, "B");
    let side = bs.iter().find(|b| b.3[0] < 30.0).unwrap_or_else(|| panic!("{bs:?}"));
    assert!((side.3[1] + 20.0).abs() < 1e-6, "B's end at the origin stops on A's face y = −20: {side:?}");
    assert_eq!(named(&mut s, "A"), before);
}

#[test]
fn a_mitre_frame_fits_its_free_ends_and_none_never_does() {
    // Fit Ends on a frame that isn't Auto fits the ends at no corner (B's line has two), and leaves its corners as
    // stage 1 builds them; a None frame never fits. So B runs y 20…380 under Mitre, 0…400 under None.
    for (corner, lo, hi) in [("mitre", 20.0, 380.0), ("none", 0.0, 400.0)] {
        let mut s = Session::default();
        earlier_frame(&mut s);
        run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Later"}));
        run(&mut s, "sketch.line", json!({"points": [[300, 0], [300, 400]]}));
        run(&mut s, "sketch.finish", json!({}));
        run(&mut s, "frame.member", json!({"sketch": "Later", "profile": square_of(25.0), "corner": corner, "fit_ends": true, "body_name": "B"}));
        assert_b_runs(&mut s, lo, hi);
    }
}

/// The overlap (mm³) between the bodies whose names start with `prefix`.
fn overlap(s: &mut Session, prefix: &str) -> f64 {
    let names: Vec<String> = named(s, prefix).into_iter().map(|b| b.0).collect();
    let i = run(s, "inspect.interference", json!({"bodies": names}));
    i["interferences"].as_array().map_or(0.0, |all| all.iter().map(|x| f(&x["volume_mm3"])).sum())
}

#[test]
fn ends_at_a_free_corner_are_fitted_to_each_other_too() {
    // An upright (300, 0)…(300, 300) and a 45° diagonal (300, 0)…(500, 200), 20 mm, leave one node inside A's
    // bottom tube: 135° from in line, Auto would mitre them, but the node is Free. Fitted to A alone, both stop at
    // y = 20, where the upright spans x 290…310 and the diagonal x 305.9…334.1 (its centre 320, half-width
    // 10 / cos 45°): they overlap up to y ≈ 24. The later laid of the two is also fitted to the earlier one.
    let mut s = Session::default();
    earlier_frame(&mut s);
    later_frame(&mut s, 0.0, &[[[300.0, 0.0], [300.0, 300.0]], [[300.0, 0.0], [500.0, 200.0]]], 20.0, true);
    let bs = named(&mut s, "B");
    assert_eq!(bs.len(), 2, "{bs:?}");
    for b in &bs {
        assert!(b.2[1] >= 20.0 - 1e-6, "{b:?}");
    }
    let o = overlap(&mut s, "B");
    assert!(o < 1e-3, "the braces overlap by {o} mm³: {bs:?}");
}

/// Construction points named after each `(name, point)`, for pairs to join.
fn points(s: &mut Session, at: &[(&str, [f64; 3])]) {
    for (name, p) in at {
        run(s, "construct.point.vertex", json!({"point": {"point": p}, "name": name}));
    }
}

/// A pair from construction point `a` to `b`, as `pairs` takes it.
fn pair(a: &str, b: &str) -> Value {
    json!({"a": {"construct_point": a}, "b": {"construct_point": b}})
}

#[test]
fn pair_braces_meeting_inside_an_earlier_tube_are_fitted_not_chained() {
    // Ruling A15: the braces of `ends_at_a_free_corner_are_fitted_to_each_other_too` as point pairs, tubes in no
    // sketch plane. Their node (300, 0, 0) lies inside A's bottom tube: the chain corner there is Free, not mitred
    // inside the tube, and both ends are fitted to A and to each other.
    let mut s = Session::default();
    earlier_frame(&mut s);
    points(&mut s, &[("N", [300.0, 0.0, 0.0]), ("U", [300.0, 300.0, 0.0]), ("D", [500.0, 200.0, 0.0])]);
    let pairs = json!([pair("N", "U"), pair("N", "D")]);
    run(&mut s, "frame.member", json!({"pairs": pairs, "profile": square_of(20.0), "corner": "auto", "fit_ends": true, "body_name": "B"}));
    let bs = named(&mut s, "B");
    assert_eq!(bs.len(), 2, "{bs:?}");
    for b in &bs {
        assert!(b.2[1] >= 20.0 - 1e-6, "{b:?}");
    }
    let o = overlap(&mut s, "B");
    assert!(o < 1e-3, "the braces overlap by {o} mm³: {bs:?}");
}

#[test]
fn pairs_ending_on_their_own_frames_tube_are_fitted_to_it_not_chained() {
    // Ruling A15 (finding 6): the same two pair braces in the rectangle's own Frame Member, under Auto. Their node is
    // on the bottom tube's middle, inside it: Free, so each brace is fitted to the bottom tube (laid first) and the
    // later one to the earlier, not mitred inside the tube. No tube of the frame overlaps another.
    let mut s = Session::default();
    rectangle_frame(&mut s);
    points(&mut s, &[("N", [300.0, 0.0, 0.0]), ("U", [300.0, 300.0, 0.0]), ("D", [500.0, 200.0, 0.0])]);
    let pairs = json!([pair("N", "U"), pair("N", "D")]);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "pairs": pairs, "profile": tube(), "corner": "auto", "fit_ends": true, "body_name": "B"}));
    let bs = named(&mut s, "B");
    assert_eq!(bs.len(), 6, "{bs:?}");
    let braces: Vec<&Measured> = bs.iter().filter(|b| b.3[1] < 350.0 && b.2[0] > 250.0 && b.3[0] < 560.0).collect();
    assert_eq!(braces.len(), 2, "{bs:?}");
    for b in braces {
        assert!(b.2[1] >= 20.0 - 1e-6, "{b:?}");
    }
    let o = overlap(&mut s, "B");
    assert!(o < 1e-3, "the frame's tubes overlap by {o} mm³: {bs:?}");
}

#[test]
fn a_mitre_frame_fits_its_free_ends_to_its_own_tubes_in_drawing_order() {
    // Ruling A14: with no earlier Frame Member, the Mitre rectangle's cross-member (300, 0)…(300, 400) has two free
    // ends on the rails' middles. Each is fitted to the rail laid before it, in drawing order, not pick order: the
    // cross-member runs y 20…380 whichever order the lines are picked in, and every body comes out the same.
    let build = |reverse: bool| {
        let mut s = Session::default();
        cross_member_frame(&mut s);
        let mut lines: Vec<String> = sketch_lines(&s, "Frame").into_iter().map(|l| l.0).collect();
        if reverse {
            lines.reverse();
        }
        let names: serde_json::Map<String, Value> = lines.iter().map(|l| (l.clone(), json!(l))).collect();
        run(
            &mut s,
            "frame.member",
            json!({"sketch": "Frame", "profile": tube(), "corner": "mitre", "fit_ends": true, "lines": lines, "names": names}),
        );
        built(&mut s)
    };
    let ((drawn, drawn_faces), (picked, picked_faces)) = (build(false), build(true));
    assert_eq!(drawn.keys().collect::<Vec<_>>(), picked.keys().collect::<Vec<_>>());
    for (line, (v, lo, hi)) in &drawn {
        let (pv, plo, phi) = picked[line];
        assert!(rel(pv, *v) < 1e-9 && near3(plo, *lo) && near3(phi, *hi), "{line}: drawn {drawn:?} picked {picked:?}");
    }
    assert_eq!(drawn_faces, picked_faces);
    let cross = drawn.values().find(|b| b.1[0] > 250.0 && b.2[0] < 350.0).unwrap_or_else(|| panic!("{drawn:?}"));
    assert!(near3(cross.1, [280.0, 20.0, -20.0]) && near3(cross.2, [320.0, 380.0, 20.0]), "{cross:?}");
    assert!(rel(cross.0, tube_area() * 360.0) < MEASURED, "{cross:?} vs {}", tube_area() * 360.0);
}

#[test]
fn a_corner_on_an_earlier_frames_corner_is_free() {
    // B's corner at (0, 0) is a corner of A, where A's two tubes meet: on both tubes' end or mitre faces, inside
    // neither. It still lies in A, so it is Free: the diagonal (0, 0)…(300, 200) is fitted to A's two tubes, which
    // fill x < 20 and y < 20 round there (min x and y 20). Under Mitre A fills the whole corner square, so B's
    // other tube, (0, 0)…(0, −300), stops on A's outer face y = −20.
    for corner in ["mitre", "none"] {
        let mut s = Session::default();
        rectangle_frame(&mut s);
        run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": corner, "name": "A", "body_name": "A"}));
        later_frame(&mut s, 0.0, &[[[0.0, 0.0], [300.0, 200.0]], [[0.0, 0.0], [0.0, -300.0]]], 20.0, true);
        let bs = named(&mut s, "B");
        let diagonal = bs.iter().find(|b| b.3[0] > 200.0).unwrap_or_else(|| panic!("{bs:?}"));
        assert!(diagonal.2[0] >= 20.0 - 1e-6 && diagonal.2[1] >= 20.0 - 1e-6, "{corner}: {diagonal:?}");
        if corner == "mitre" {
            let down = bs.iter().find(|b| b.3[0] < 20.0).unwrap_or_else(|| panic!("{bs:?}"));
            assert!((down.3[1] + 20.0).abs() < 1e-6, "{down:?}");
        }
    }
}

/// A Locost-sized design (about 60 tubes over four Frame Members, all Auto with fit_ends), timed through a full
/// evaluation, which it prints (no time is asserted). Run on demand: `cargo test ... locost_sized_design_timing --
/// --ignored --nocapture`.
#[test]
#[ignore = "timing check, run on demand"]
fn locost_sized_design_timing() {
    let mut s = Session::default();
    let frame = |s: &mut Session, name: &str, plane: &str, offset: f64| {
        let mut sk = json!({"plane": plane, "name": name});
        if offset != 0.0 {
            sk["offset"] = json!(offset);
        }
        run(s, "sketch.create", sk);
        run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [1000, 600]}));
        for x in [200, 400, 600, 800] {
            run(s, "sketch.line", json!({"points": [[x, 0], [x, 600]]}));
        }
        run(s, "sketch.finish", json!({}));
        run(
            s,
            "frame.member",
            json!({"sketch": name, "profile": json!({"kind": "rect_tube", "width": 25, "height": 25, "wall": 2}), "corner": "auto", "fit_ends": true, "body_name": name}),
        );
    };
    frame(&mut s, "Left", "XZ", 0.0);
    frame(&mut s, "Right", "XZ", 400.0);
    frame(&mut s, "Floor", "XY", 0.0);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Braces", "offset": 25}));
    let zigzag: Vec<Value> = (0..=36).map(|i| json!([i as f64 * 1000.0 / 36.0, if i % 2 == 0 { 0 } else { 400 }])).collect();
    run(&mut s, "sketch.line", json!({"points": zigzag}));
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "frame.member",
        json!({"sketch": "Braces", "profile": json!({"kind": "rect_tube", "width": 20, "height": 20, "wall": 2}), "corner": "auto", "fit_ends": true, "body_name": "Brace"}),
    );
    let json_doc = s.doc.to_json();
    let start = std::time::Instant::now();
    let mut reopened = Session::new(solidcraft_doc::Document::from_json(&json_doc).unwrap());
    let n = bodies(&mut reopened).len();
    println!("LOCOST tubes={n} full evaluation={:.2?}", start.elapsed());
    assert!(n >= 50, "{n}");
}
