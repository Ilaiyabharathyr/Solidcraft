use solidcore_record::Vec3;

use super::*;

fn seg(id: &str, a: (f64, f64), b: (f64, f64)) -> (String, Vec3, Vec3) {
    (id.to_string(), Vec3::new(a.0, a.1, 0.0), Vec3::new(b.0, b.1, 0.0))
}

/// Every member on the XY sketch plane, normal +Z, x +X.
fn xy(segments: &[(String, Vec3, Vec3)]) -> Vec<MemberPlane> {
    vec![Some((Vec3::Z, Vec3::X)); segments.len()]
}

/// Drawn in the order given.
fn in_order(segments: &[(String, Vec3, Vec3)]) -> Vec<usize> {
    (0..segments.len()).collect()
}

/// The ids in laying order, each with whether it is outer.
fn laid(segments: &[(String, Vec3, Vec3)]) -> Vec<(String, bool)> {
    ranks(segments, &xy(segments), &in_order(segments)).iter().map(|r| (segments[r.member].0.clone(), r.outer)).collect()
}

fn ids(v: &[(&str, bool)]) -> Vec<(String, bool)> {
    v.iter().map(|(s, o)| (s.to_string(), *o)).collect()
}

/// The 600×400 rectangle, drawn anticlockwise from the origin: bottom, right, top, left.
fn rectangle() -> Vec<(String, Vec3, Vec3)> {
    vec![
        seg("bottom", (0.0, 0.0), (600.0, 0.0)),
        seg("right", (600.0, 0.0), (600.0, 400.0)),
        seg("top", (600.0, 400.0), (0.0, 400.0)),
        seg("left", (0.0, 400.0), (0.0, 0.0)),
    ]
}

/// The rectangle with a cross-member at x = 300 and a diagonal from (0,0) to (300,400).
fn crossed() -> Vec<(String, Vec3, Vec3)> {
    let mut s = rectangle();
    s.push(seg("cross", (300.0, 0.0), (300.0, 400.0)));
    s.push(seg("diagonal", (0.0, 0.0), (300.0, 400.0)));
    s
}

#[test]
fn a_rectangles_long_sides_lay_first() {
    assert_eq!(laid(&rectangle()), ids(&[("bottom", true), ("top", true), ("right", true), ("left", true)]));
}

#[test]
fn a_squares_horizontal_sides_lay_first() {
    // Drawn verticals first, so only the tie on the sketch's X axis puts the horizontals first.
    let segs = vec![
        seg("right", (400.0, 0.0), (400.0, 400.0)),
        seg("left", (0.0, 400.0), (0.0, 0.0)),
        seg("bottom", (0.0, 0.0), (400.0, 0.0)),
        seg("top", (400.0, 400.0), (0.0, 400.0)),
    ];
    assert_eq!(laid(&segs), ids(&[("bottom", true), ("top", true), ("right", true), ("left", true)]));
}

#[test]
fn a_rectangle_with_a_cross_member_and_a_diagonal_lays_in_order() {
    let expected = [("bottom", true), ("top", true), ("right", true), ("left", true), ("cross", false), ("diagonal", false)];
    assert_eq!(laid(&crossed()), ids(&expected));
}

#[test]
fn an_l_shape_has_six_outer_tubes_and_its_inner_tube_last() {
    let segs = vec![
        seg("a", (0.0, 0.0), (600.0, 0.0)),
        seg("b", (600.0, 0.0), (600.0, 200.0)),
        seg("c", (600.0, 200.0), (200.0, 200.0)),
        seg("d", (200.0, 200.0), (200.0, 600.0)),
        seg("e", (200.0, 600.0), (0.0, 600.0)),
        seg("f", (0.0, 600.0), (0.0, 0.0)),
        seg("inner", (0.0, 0.0), (200.0, 200.0)),
    ];
    let out = laid(&segs);
    assert_eq!(out.len(), 7);
    assert!(out[..6].iter().all(|(_, outer)| *outer), "{out:?}");
    assert_eq!(out[6], ("inner".to_string(), false));
    // Longest first, ties on the X axis then drawn order: a (600, horizontal), f (600), c, d, e, b.
    let order: Vec<&str> = out.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(order, ["a", "f", "c", "d", "e", "b", "inner"]);
}

#[test]
fn a_tube_sticking_out_is_laid_last_and_is_not_outer() {
    let mut segs = rectangle();
    segs.push(seg("out", (600.0, 200.0), (800.0, 200.0)));
    assert_eq!(laid(&segs), ids(&[("bottom", true), ("top", true), ("right", true), ("left", true), ("out", false)]));
}

#[test]
fn a_u_and_an_h_lay_longest_first() {
    let u = vec![seg("left", (0.0, 400.0), (0.0, 0.0)), seg("bottom", (0.0, 0.0), (600.0, 0.0)), seg("right", (600.0, 0.0), (600.0, 300.0))];
    assert_eq!(laid(&u), ids(&[("bottom", false), ("left", false), ("right", false)]));
    // The H: the bar's ends on the middles of the two uprights.
    let h = vec![seg("bar", (0.0, 300.0), (400.0, 300.0)), seg("left", (0.0, 0.0), (0.0, 600.0)), seg("right", (400.0, 0.0), (400.0, 600.0))];
    assert_eq!(laid(&h), ids(&[("left", false), ("right", false), ("bar", false)]));
}

#[test]
fn two_groups_lay_by_their_longest_tube() {
    let p = |x: f64, y: f64, z: f64| Vec3::new(x, y, z);
    let segs = vec![
        // A 400×400 square in XY.
        ("s1".to_string(), p(0.0, 0.0, 0.0), p(400.0, 0.0, 0.0)),
        ("s2".to_string(), p(400.0, 0.0, 0.0), p(400.0, 400.0, 0.0)),
        ("s3".to_string(), p(400.0, 400.0, 0.0), p(0.0, 400.0, 0.0)),
        ("s4".to_string(), p(0.0, 400.0, 0.0), p(0.0, 0.0, 0.0)),
        // A 600×300 rectangle in XZ, sharing only the origin.
        ("r1".to_string(), p(0.0, 0.0, 0.0), p(-600.0, 0.0, 0.0)),
        ("r2".to_string(), p(-600.0, 0.0, 0.0), p(-600.0, 0.0, 300.0)),
        ("r3".to_string(), p(-600.0, 0.0, 300.0), p(0.0, 0.0, 300.0)),
        ("r4".to_string(), p(0.0, 0.0, 300.0), p(0.0, 0.0, 0.0)),
    ];
    let xz = Some((-Vec3::Y, Vec3::X));
    let planes = vec![Some((Vec3::Z, Vec3::X)), Some((Vec3::Z, Vec3::X)), Some((Vec3::Z, Vec3::X)), Some((Vec3::Z, Vec3::X)), xz, xz, xz, xz];
    let out = ranks(&segs, &planes, &in_order(&segs));
    let got: Vec<(&str, usize, bool)> = out.iter().map(|r| (segs[r.member].0.as_str(), r.group, r.outer)).collect();
    assert_eq!(
        got,
        [("r1", 0, true), ("r3", 0, true), ("r2", 0, true), ("r4", 0, true), ("s1", 1, true), ("s3", 1, true), ("s2", 1, true), ("s4", 1, true)]
    );
}

#[test]
fn members_in_no_plane_come_last_longest_first() {
    let mut segs = rectangle();
    segs.push(("short".to_string(), Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 100.0)));
    segs.push(("long".to_string(), Vec3::new(600.0, 0.0, 0.0), Vec3::new(600.0, 0.0, 300.0)));
    let mut planes = xy(&segs);
    planes[4] = None;
    planes[5] = None;
    let out = ranks(&segs, &planes, &in_order(&segs));
    let got: Vec<(&str, usize, bool)> = out.iter().map(|r| (segs[r.member].0.as_str(), r.group, r.outer)).collect();
    assert_eq!(got[4..], [("long", 1, false), ("short", 1, false)]);
}

#[test]
fn pick_order_never_changes_the_order() {
    let segs = crossed();
    let drawn = in_order(&segs);
    let order = |perm: &[usize]| {
        let s: Vec<_> = perm.iter().map(|&i| segs[i].clone()).collect();
        let d: Vec<_> = perm.iter().map(|&i| drawn[i]).collect();
        ranks(&s, &xy(&s), &d).iter().map(|r| (s[r.member].0.clone(), r.group, r.outer, r.reverse)).collect::<Vec<_>>()
    };
    let expected = order(&[0, 1, 2, 3, 4, 5]);
    assert_eq!(expected.len(), 6);
    for perm in [[5, 4, 3, 2, 1, 0], [2, 5, 0, 3, 1, 4], [4, 0, 5, 1, 3, 2]] {
        assert_eq!(order(&perm), expected, "{perm:?}");
    }
}

#[test]
fn outer_tubes_run_anticlockwise() {
    let mut segs = rectangle();
    segs[0] = seg("bottom", (600.0, 0.0), (0.0, 0.0));
    let out = ranks(&segs, &xy(&segs), &in_order(&segs));
    let reverse: Vec<(&str, bool)> = out.iter().map(|r| (segs[r.member].0.as_str(), r.reverse)).collect();
    assert_eq!(reverse, [("bottom", true), ("top", false), ("right", false), ("left", false)]);
    // Seen from behind (normal -Z), the same lines run the other way round.
    let behind = vec![Some((-Vec3::Z, Vec3::X)); segs.len()];
    let out = ranks(&segs, &behind, &in_order(&segs));
    assert!(out.iter().all(|r| r.reverse == (segs[r.member].0 != "bottom")), "{out:?}");
}

#[test]
fn hostile_members_never_panic_and_each_is_laid_once() {
    let nan = f64::NAN;
    let segs = vec![
        seg("a", (0.0, 0.0), (600.0, 0.0)),
        seg("nan", (nan, 0.0), (1.0, 1.0)),
        seg("point", (5.0, 5.0), (5.0, 5.0)),
        seg("huge", (-1e300, 0.0), (1e300, 0.0)),
        seg("b", (600.0, 0.0), (0.0, 0.0)),
    ];
    let mut planes = xy(&segs);
    planes[0] = Some((Vec3::new(nan, 0.0, 1.0), Vec3::X));
    let out = ranks(&segs, &planes, &[]);
    let mut members: Vec<usize> = out.iter().map(|r| r.member).collect();
    members.sort_unstable();
    assert_eq!(members, [0, 1, 2, 3, 4]);
}

#[test]
fn a_bridge_between_two_closed_shapes_is_laid_last_and_is_not_outer() {
    let square = |n: &str, x0: f64| {
        vec![
            seg(&format!("{n}1"), (x0, 0.0), (x0 + 200.0, 0.0)),
            seg(&format!("{n}2"), (x0 + 200.0, 0.0), (x0 + 200.0, 200.0)),
            seg(&format!("{n}3"), (x0 + 200.0, 200.0), (x0, 200.0)),
            seg(&format!("{n}4"), (x0, 200.0), (x0, 0.0)),
        ]
    };
    let mut segs = vec![seg("bridge", (200.0, 100.0), (400.0, 100.0))];
    segs.extend(square("a", 0.0));
    segs.extend(square("b", 400.0));
    let out = laid(&segs);
    assert_eq!(out.len(), 9);
    assert!(out[..8].iter().all(|(_, outer)| *outer), "{out:?}");
    assert_eq!(out[8], ("bridge".to_string(), false));
}

#[test]
fn a_tube_crossing_the_outline_is_in_its_group() {
    let mut segs = rectangle();
    segs.push(seg("across", (-100.0, 200.0), (100.0, 200.0)));
    let out = ranks(&segs, &xy(&segs), &in_order(&segs));
    let last = out.last().map(|r| (segs[r.member].0.as_str(), r.group, r.outer));
    assert_eq!(out.len(), 5);
    assert_eq!(last, Some(("across", 0, false)));
}
