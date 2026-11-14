//! Frame corners (#4003): where exactly two members of a frame end, and the planes their ends are
//! cut on. Pure geometry: the members are built long enough ([`extension`]) and cut by the caller.
//!
//! At corner P each member's direction away from P is `w`. Mitre cuts both on the bisecting plane;
//! Butt runs one member, T, through, cut flush with the other's outer face, and cuts the other, U,
//! at T's near face; Coped Butt cuts T as Butt and leaves U to be coped to T's shape.

use std::collections::HashMap;
use std::f64::consts::PI;

use serde::{Deserialize, Serialize};
use solidcore_record::{Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::prelude::*;

use crate::{DocError, Result};

/// How two members are finished where they meet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CornerType {
    #[default]
    None,
    Mitre,
    Butt,
    CopedButt,
}

impl CornerType {
    pub fn is_none(&self) -> bool {
        *self == CornerType::None
    }
    /// Whether Flip means anything: which member runs through.
    pub fn flips(&self) -> bool {
        matches!(self, CornerType::Butt | CornerType::CopedButt)
    }
}

/// One corner finished its own way, named by the lines of the two members that meet there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CornerOverride {
    pub members: [String; 2],
    pub corner: CornerType,
    #[serde(default)]
    pub flip: bool,
}

/// How close (mm) two line ends must be to meet.
pub const CORNER_TOL: f64 = 1e-3;
/// Members within this length (mm) of each other are equally long.
const EQUAL_LENGTH: f64 = 1.0;
/// Members within this angle (degrees) of a straight run on are in line: nothing to cut.
const IN_LINE_DEG: f64 = 0.01;
/// The sharpest interior angle (degrees) a corner can be finished at.
const MIN_INTERIOR_DEG: f64 = 5.0;

/// Where exactly two members end. `ends[k] = (member index, at_start)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Corner {
    pub point: Vec3,
    pub ends: [(usize, bool); 2],
}

/// A cut keeps the side where dot(X - origin, normal) >= 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    pub origin: Vec3,
    pub normal: Vec3,
}

/// The points where exactly two members end and no third member ends or passes, ordered by the
/// lower member index, then the other.
pub fn find_corners(segments: &[(String, Vec3, Vec3)]) -> Vec<Corner> {
    let ends: Vec<(usize, bool, Vec3)> =
        segments.iter().enumerate().flat_map(|(m, (_, a, b))| [(m, true, *a), (m, false, *b)]).filter(|e| e.2.is_finite()).collect();
    // Ends within CORNER_TOL of each other fall in the same or neighbouring cells.
    let cell = |p: Vec3| ((p.x / CORNER_TOL).floor() as i64, (p.y / CORNER_TOL).floor() as i64, (p.z / CORNER_TOL).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (k, e) in ends.iter().enumerate() {
        grid.entry(cell(e.2)).or_default().push(k);
    }
    let near = |k: usize, p: Vec3| {
        let (x, y, z) = cell(p);
        let mut out = Vec::new();
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                for dz in -1..=1i64 {
                    let key = (x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz));
                    let hits = grid.get(&key).into_iter().flatten();
                    out.extend(hits.filter(|&&j| j != k && ends.get(j).is_some_and(|e| e.2.dist(p) <= CORNER_TOL)));
                }
            }
        }
        out
    };
    let mut corners = Vec::new();
    for (i, &(mi, si, pi)) in ends.iter().enumerate() {
        let &[&j] = near(i, pi).as_slice() else { continue };
        let Some(&(mj, sj, pj)) = ends.get(j) else { continue };
        if j < i || mi == mj || near(j, pj) != [&i] {
            continue;
        }
        let point = (pi + pj) * 0.5;
        // ponytail: every corner checks every member, O(members × corners), fine under MAX_MEMBERS; grid the segments if it shows.
        let crossed = segments.iter().enumerate().any(|(m, (_, a, b))| m != mi && m != mj && point.dist_to_segment(*a, *b) <= CORNER_TOL);
        if !crossed {
            corners.push(Corner { point, ends: [(mi, si), (mj, sj)] });
        }
    }
    corners.sort_by_key(|c| {
        let [(a, sa), (b, sb)] = c.ends;
        (a, b, sa, sb)
    });
    corners
}

/// "Corner at (x, y, z) is {what}", each coordinate to 2 decimals without trailing zeros.
pub fn corner_error(p: Vec3, what: &str) -> DocError {
    let r = |c: f64| (c * 100.0).round() / 100.0 + 0.0;
    DocError::Invalid(format!("Corner at ({}, {}, {}) is {what}", r(p.x), r(p.y), r(p.z)))
}

/// Member `m`'s end at `corner`: (m, at_start).
pub fn end_at(corner: &Corner, m: usize) -> Result<(usize, bool)> {
    corner.ends.iter().copied().find(|e| e.0 == m).ok_or_else(|| DocError::Invalid(format!("member {m} is not at the corner it is coped at")))
}

/// A member's unit direction away from the corner at its end `(member, at_start)`.
pub fn away(segments: &[(String, Vec3, Vec3)], (m, at_start): (usize, bool)) -> Option<Vec3> {
    let (_, a, b) = segments.get(m)?;
    (if at_start { *b - *a } else { *a - *b }).normalized()
}

/// Index into `corner.ends` (0 or 1) of the member that runs through, unflipped: the longer, then
/// the one nearer the sketch's X axis, then the one drawn first: `drawn[m]` is member m's place in
/// the sketch's drawing order (a member missing from it ranks by its index), never the pick order.
pub fn through(segments: &[(String, Vec3, Vec3)], corner: &Corner, sketch_x: Option<Vec3>, drawn: &[usize]) -> usize {
    let [e0, e1] = corner.ends;
    let pick = |first: bool| if first { 0 } else { 1 };
    let length = |m: usize| segments.get(m).map_or(0.0, |(_, a, b)| a.dist(*b));
    let (l0, l1) = (length(e0.0), length(e1.0));
    if (l0 - l1).abs() > EQUAL_LENGTH {
        return pick(l0 > l1);
    }
    if let Some(x) = sketch_x {
        let along = |e| away(segments, e).map_or(0.0, |w| w.dot(x).abs());
        let (c0, c1) = (along(e0), along(e1));
        if (c0 - c1).abs() > 1e-9 {
            return pick(c0 > c1);
        }
    }
    let rank = |m: usize| drawn.get(m).copied().unwrap_or(m);
    pick((rank(e0.0), e0.0) <= (rank(e1.0), e1.0))
}

/// The interior angle (radians) at a corner: π for in line.
pub fn interior(segments: &[(String, Vec3, Vec3)], corner: &Corner) -> f64 {
    let [e0, e1] = corner.ends;
    match (away(segments, e0), away(segments, e1)) {
        (Some(a), Some(b)) => a.dot(b).clamp(-1.0, 1.0).acos(),
        _ => PI,
    }
}

/// How far a placed section reaches from `point` along unit `dir`: the max over the regions'
/// outer loops (lines' vertices and arcs' extremes), each 2D point p mapped as
/// plane.origin + plane.x*p.x + plane.y*p.y.
pub fn reach(plane: &Plane, regions: &[Region2], point: Vec3, dir: Vec3) -> f64 {
    let d = Vec2::new(plane.x.dot(dir), plane.y.dot(dir));
    let base = (plane.origin - point).dot(dir);
    let mut best = f64::NEG_INFINITY;
    for seg in regions.iter().flat_map(|r| &r.outer.segs) {
        let mut take = |p: Vec2| best = best.max(base + p.dot(d));
        match *seg {
            Seg2::Line { a, b } => {
                take(a);
                take(b);
            }
            Seg2::Arc { center, radius, start, sweep } => {
                take(seg.start());
                take(seg.end());
                // The arc's furthest point along d, if the arc passes it.
                let (lo, hi) = if sweep >= 0.0 { (start, start + sweep) } else { (start + sweep, start) };
                let t = d.y.atan2(d.x);
                let t = t + ((lo - t) / std::f64::consts::TAU).ceil() * std::f64::consts::TAU;
                if t <= hi {
                    take(center + Vec2::from_angle(t) * radius);
                }
            }
            _ => seg.polyline(1e-3).into_iter().for_each(take),
        }
    }
    best
}

/// The cuts at one corner: (member index, at_start, Cut) for each member it cuts. Coped's U gets
/// no plane cut here; it returns T's cut only, plus the pair (t, u) for coping. `placed(member,
/// at)` is the member's section plane placed square to its line at `at`.
pub fn corner_cuts(
    kind: CornerType,
    flip: bool,
    corner: &Corner,
    segments: &[(String, Vec3, Vec3)],
    placed: &dyn Fn(usize, Vec3) -> Plane,
    regions: &[Region2],
    sketch_x: Option<Vec3>,
    drawn: &[usize],
) -> Result<(Vec<(usize, bool, Cut)>, Option<(usize, usize)>)> {
    let angle = interior(segments, corner);
    if kind.is_none() || angle > PI - IN_LINE_DEG.to_radians() {
        return Ok((Vec::new(), None));
    }
    let p = corner.point;
    if angle < MIN_INTERIOR_DEG.to_radians() {
        return Err(corner_error(p, "too sharp to finish"));
    }
    let [e0, e1] = corner.ends;
    let dir = |e: (usize, bool)| away(segments, e).ok_or_else(|| DocError::Invalid(format!("member {} of a corner has no length", e.0)));
    if kind == CornerType::Mitre {
        let n = (dir(e0)? - dir(e1)?).normalized().ok_or_else(|| DocError::Invalid("a mitre corner has no bisector".into()))?;
        return Ok((vec![(e0.0, e0.1, Cut { origin: p, normal: n }), (e1.0, e1.1, Cut { origin: p, normal: -n })], None));
    }
    let (te, ue) = if (through(segments, corner, sketch_x, drawn) == 0) != flip { (e0, e1) } else { (e1, e0) };
    let (t, u) = (-dir(te)?, dir(ue)?);
    // d made square to `axis`: the normal of a side face of the member along axis.
    let across =
        |d: Vec3, axis: Vec3| (d - axis * d.dot(axis)).normalized().ok_or_else(|| DocError::Invalid("a butt corner has no side to meet".into()));
    let finite = |r: f64| if r.is_finite() { Ok(r) } else { Err(DocError::Invalid("the member profile has no area".into())) };
    // T stops at U's outer face, U at T's near face.
    let m = across(t, u)?;
    let t_cut = Cut { origin: p + m * finite(reach(&placed(ue.0, p), regions, p, m))?, normal: -m };
    if kind == CornerType::CopedButt {
        return Ok((vec![(te.0, te.1, t_cut)], Some((te.0, ue.0))));
    }
    let n = across(u, t)?;
    let u_cut = Cut { origin: p + n * finite(reach(&placed(te.0, p), regions, p, n))?, normal: n };
    Ok((vec![(te.0, te.1, t_cut), (ue.0, ue.1, u_cut)], None))
}

/// How far past its corner a member must be built for `cut` to cross its whole section, plus
/// 1 mm (never less than 1 mm): `plane` is the member's section placed at the corner `point`, and
/// `away` its unit direction away from the corner. A section point q meets the cut plane
/// ((q - origin)·n) / (away·n) back from the corner, furthest for the q that reaches furthest
/// along n. `None` when the section has no area or the cut runs along the member.
pub fn extension(plane: &Plane, regions: &[Region2], point: Vec3, away: Vec3, cut: &Cut) -> Option<f64> {
    let along = away.dot(cut.normal);
    let back = (reach(plane, regions, point, cut.normal) - (cut.origin - point).dot(cut.normal)) / along;
    (along > 1e-9 && back.is_finite()).then(|| back.max(0.0) + 1.0)
}

#[cfg(test)]
#[path = "frame_corners_tests.rs"]
mod tests;
