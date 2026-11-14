//! Cut to fit (#4003): a tube end cut to the tubes it meets, as a fabricator fits it. The tool is the other tube
//! with its hole filled and its corner radii squared off, built from its section ([`squared_outline`]): a fitted
//! end stops flat on a flat side, and is an exact fishmouth on a round tube. Exact geometry only, never a mesh.
// The Auto build fits ends with these from the next task on.
#![cfg_attr(not(test), allow(dead_code))]

use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{self as kernel, BoolOp, PointClass};

use crate::frame_corners::Cut;
use crate::lineage::{Lineage, Traced, numbered_tool};
use crate::{DocError, Result};

/// The section's outer outlines with every corner arc that sits between two straight sides replaced by the
/// corner they meet at; holes dropped. A round section (no straight sides) keeps its arcs.
pub fn squared_outline(regions: &[Region2]) -> Vec<Region2> {
    regions.iter().map(|r| Region2 { outer: squared(&r.outer), holes: Vec::new() }).collect()
}

/// `outline` with each arc between two lines replaced by the point where the lines meet.
fn squared(outline: &Loop2) -> Loop2 {
    let segs = &outline.segs;
    let n = segs.len();
    let line = |k: usize| match segs.get(k.checked_rem(n)?) {
        Some(Seg2::Line { a, b }) => Some((*a, *b)),
        _ => None,
    };
    // The corner replacing segment k, when it is an arc between two lines that meet.
    let corner = |k: usize| match segs.get(k.checked_rem(n)?) {
        Some(Seg2::Arc { .. }) => meet(line(k + n - 1)?, line(k + 1)?),
        _ => None,
    };
    let segs = segs
        .iter()
        .enumerate()
        .filter_map(|(k, seg)| match *seg {
            Seg2::Line { a, b } => Some(Seg2::Line { a: corner(k + n - 1).unwrap_or(a), b: corner(k + 1).unwrap_or(b) }),
            Seg2::Arc { .. } if corner(k).is_some() => None,
            _ => Some(*seg),
        })
        .collect();
    Loop2 { segs }
}

/// Where the lines through (a, b) and (c, d) meet; `None` for parallel lines.
fn meet((a, b): (Vec2, Vec2), (c, d): (Vec2, Vec2)) -> Option<Vec2> {
    let (u, v) = (b - a, d - c);
    let across = u.cross(v);
    if across.abs() <= 1e-12 * u.len() * v.len() {
        return None;
    }
    Some(a + u * ((c - a).cross(v) / across))
}

/// A tube's cutting tool: the squared outline extruded on `plane` from `start` to `end` (as the tube is), then
/// the same plane cuts as the tube (stage 1 Cut list), traced as a numbered tool.
pub fn fit_tool(plane: &Plane, regions: &[Region2], start: f64, end: f64, cuts: &[Cut]) -> Result<Traced> {
    let mut bodies = kernel::extrude(plane, &squared_outline(regions), start, end)?;
    let (Some(mut body), true) = (bodies.pop(), bodies.is_empty()) else {
        return Err(DocError::Invalid("a tube's fitting tool is not one solid".into()));
    };
    for cut in cuts {
        let plane = Plane::from_normal(cut.origin, cut.normal).ok_or_else(|| DocError::Invalid("a tube's corner cut has no plane".into()))?;
        body = kernel::half_space(&body, &plane, true)?.0.ok_or_else(|| DocError::Invalid("a tube's fitting tool was cut away completely".into()))?;
    }
    Ok(numbered_tool(body))
}

/// `member` with its end at `end_point` (its line's end, `toward` pointing out of the member) fitted to `tools`:
/// only tools whose exact bounds come within the member's section diagonal of the end take part; the piece
/// holding `middle` (a point inside the member's material near its line's middle) is kept; unchanged when no
/// tool reaches the end.
pub fn fit_end(member: &Traced, end_point: Vec3, toward: Vec3, middle: Vec3, tools: &[&Traced], id: &str) -> Result<Traced> {
    let gone = || DocError::Invalid(format!("the member on line `{id}` was cut away completely"));
    let toward = toward.normalized().ok_or_else(|| DocError::Invalid(format!("the member on line `{id}` has no direction")))?;
    let room = diagonal(&member.body, end_point, toward, middle);
    let mut fitted: Option<Traced> = None;
    for tool in tools {
        let b = tool.body.exact_bounds()?;
        if end_point.dist(end_point.max(b.min).min(b.max)) > room {
            continue;
        }
        let from = fitted.as_ref().unwrap_or(member);
        let cut = kernel::boolean(&from.body, &tool.body, BoolOp::Cut)?.ok_or_else(gone)?;
        fitted = Some(Traced::step(cut, &[from, tool]));
    }
    let Some(fitted) = fitted else { return Ok(member.clone()) };
    let lumps = fitted.body.lumps()?;
    let lineages = Lineage::lumps(&fitted.body, fitted.lineage.as_ref(), &lumps);
    for (body, lineage) in lumps.into_iter().zip(lineages) {
        if body.classify_point(middle)? != PointClass::Outside {
            return Ok(Traced { body, lineage, recipe: None });
        }
    }
    Err(gone())
}

/// The member's section diagonal at its end: twice its farthest vertex from the line through `end` along unit
/// `toward`, of the vertices nearer the end than `middle` is (all of them when none is).
fn diagonal(body: &kernel::Body, end: Vec3, toward: Vec3, middle: Vec3) -> f64 {
    let half = end.dist(middle);
    let points: Vec<Vec3> = body.record().vertices.iter().map(|v| Vec3::new(v.point[0], v.point[1], v.point[2])).collect();
    let off = |p: &Vec3| {
        let d = *p - end;
        (d - toward * d.dot(toward)).len()
    };
    let near = points.iter().filter(|p| p.dist(end) < half).map(off).fold(f64::NEG_INFINITY, f64::max);
    2.0 * if near.is_finite() { near } else { points.iter().map(off).fold(0.0, f64::max) }
}

#[cfg(test)]
#[path = "frame_fit_tests.rs"]
mod tests;
