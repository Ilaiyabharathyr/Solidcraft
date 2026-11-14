//! Earlier Frame Members' tubes as cutting tools (#4003 Goal 6, decisions 1-2): what Fit Ends to Earlier Frames
//! fits a later Frame Member's ends to.
//!
//! **How a copy records its source.** It doesn't, on the body: a copy is a new [`ModelBody`] whose `feature` is the
//! copying feature, its geometry the source's moved by `kernel::transform_matrix`.
//! - `ModelBody.source_transform` is not a copy's transform: it is where an assembly placement or a component frame
//!   puts a body (`in_frame`, placed occurrences, `body_transfer`), and it stays identity through Move, Mirror and
//!   Pattern.
//! - The lineage names the source only for face names: `moved_copy` steps the copy from
//!   `Operand::Copy { body, instance }` (the source's name), and a copied feature's own faces become
//!   `Operand::CopiedTool { feature, instance }` (`Traced::copied`). Neither keeps the transform, and a body's
//!   lineage is only the feature's own (`ModelBody.made`).
//! - The transform lives in the copying feature alone: a Move's `translate`/`rotate_axis`/`origin`/`angle` (a
//!   Move without `copy` moves the body in place, which keeps its name and its `feature`); a Pattern's
//!   `pattern_transforms`; a Mirror's `mirror_matrix` of its plane. A Pattern or Mirror of bodies copies the bodies
//!   it names, one of features (`replay`, `copied_features`) copies every body a Frame Member or a Move copy among
//!   those features made.
//!
//! So [`earlier_tools`] walks the timeline up to the feature, as evaluation did: a Frame Member's bodies start at its
//! own place; each Move, Pattern and Mirror after it moves or copies them by its own transforms. A copy may come
//! from any of the bodies its feature copied, by any of its transforms: each is a pose, (source Frame Member, matrix
//! from its frame to the body's). A body is then matched to the member and pose it is exactly: a point in that
//! member's material half way along it, placed by the pose, lies inside the body (exact point classification,
//! never a mesh). Only then is the member's tool built (once per member in a call) and placed (once per pose).
//! This also refuses a body moved some way the walk doesn't follow (Align, Rest, a transfer to another component,
//! a mirror at a placement): it is not fitted to, rather than fitted to in the wrong place.
//!
//! The tool is the source member as built ([`super::member_tool`]): its feature's members laid out and its corner
//! plane cuts made as [`super::member_section`] makes them, then filled and squared ([`frame_fit::fit_tool`]).
//! Earlier tubes and their copies are only read, never changed.

use std::collections::{BTreeMap, HashMap, HashSet};

use solidcore_record::{Region2, Vec2, Vec3};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{self as kernel, PointClass};

use super::super::{ModelState, mirror_matrix, pattern_transforms, point, val};
use super::{Axes, CutEnd, Layout, auto_plan, frame_ends, layout, member_tool, placer, section_axes};
use crate::expr::{Kind, Value};
use crate::frame::MAX_MEMBERS;
use crate::lineage::Traced;
use crate::{DocError, Document, Feature, FeatureKind, Mat, Result};

#[cfg(test)]
thread_local! {
    /// How many member tools [`Members::tool`] has built on this thread.
    pub(super) static TOOL_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A body's place: the Frame Member it is a tube of, and the matrix from that feature's frame to the body's.
type Pose = (u64, Mat);

/// Every tube made before feature `f` by a FrameMember feature, and every moved / mirrored / patterned copy of
/// one, as a cutting tool placed where that body is now: (body name, tool). Tools are built once per call.
pub fn earlier_tools(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Vec<(String, Traced)>> {
    let Some(here) = doc.features.iter().position(|g| g.id == f.id) else { return Ok(Vec::new()) };
    // Each tracked body's feature (a Frame Member, or what copied it) and the poses it may have.
    let mut tracked: BTreeMap<String, (u64, Vec<Pose>)> = BTreeMap::new();
    for g in doc.features.iter().take(here).filter(|g| !g.suppressed) {
        let made: Vec<String> = st.bodies.iter().filter(|b| b.feature == g.id).map(|b| b.name.clone()).collect();
        let (named, features, mats): (&[crate::BodyName], &[crate::FeatureId], Vec<Mat>) = match &g.kind {
            FeatureKind::FrameMember { .. } => {
                for n in made {
                    tracked.insert(n, (g.id, vec![(g.id, crate::IDENTITY)]));
                }
                continue;
            }
            FeatureKind::Move { bodies, copy: false, .. } => {
                let r = move_matrix(vals, &g.kind);
                for n in bodies {
                    if let Some((_, poses)) = tracked.get_mut(n.as_ref()) {
                        // A move the walk can't work out: those bodies' places are no longer known.
                        *poses = r.map_or_else(Vec::new, |r| poses.iter().map(|(s, m)| (*s, crate::mat_mul(&r, m))).collect());
                    }
                }
                continue;
            }
            FeatureKind::Move { bodies, .. } => (bodies.as_slice(), &[], move_matrix(vals, &g.kind).into_iter().collect()),
            FeatureKind::Pattern { features, pattern, bodies } => {
                (bodies.as_slice(), features.as_slice(), pattern_transforms(doc, vals, st, g.component, pattern).unwrap_or_default())
            }
            FeatureKind::Mirror { features, plane, bodies, combine: false, .. } => {
                let mirror = doc.resolve_plane_in(vals, plane, 0, g.component, Some(st)).map(|pl| mirror_matrix(&pl));
                (bodies.as_slice(), features.as_slice(), mirror.into_iter().collect())
            }
            _ => continue,
        };
        // The copies' poses: each source's, moved by each transform, once each.
        let sources = tracked.values().filter(|(by, _)| features.iter().any(|s| s.0 == *by));
        let sources = sources.chain(named.iter().filter_map(|n| tracked.get(n.as_ref())));
        let mut seen = HashSet::new();
        let mut poses = Vec::new();
        for (s, m) in sources.flat_map(|(_, poses)| poses) {
            for t in &mats {
                let pose = (*s, crate::mat_mul(t, m));
                if poses.len() < MAX_MEMBERS && seen.insert((pose.0, bits(&pose.1))) {
                    poses.push(pose);
                }
            }
        }
        if !poses.is_empty() {
            for n in made {
                tracked.insert(n, (g.id, poses.clone()));
            }
        }
    }
    let into_f = |component: u64| -> Option<Mat> {
        if component == f.component {
            return Some(crate::IDENTITY);
        }
        crate::mat_inverse(&doc.component_transform(f.component)).map(|inv| crate::mat_mul(&inv, &doc.component_transform(component)))
    };
    let mut members: HashMap<u64, Option<Members>> = HashMap::new();
    let mut placed: HashMap<(u64, usize, [u64; 16]), Traced> = HashMap::new();
    let mut out = Vec::new();
    for b in &st.bodies {
        let Some((_, poses)) = tracked.get(&b.name) else { continue };
        let (Some(to_f), false, Ok(bounds)) = (into_f(b.component), b.body.is_mesh(), b.body.exact_bounds()) else { continue };
        'body: for (g, m) in poses {
            // An earlier feature that no longer lays out has no tubes to fit to.
            let Some(source) = members.entry(*g).or_insert_with(|| doc.feature(*g).and_then(|g| Members::of(doc, vals, g, st).ok())).as_mut() else {
                continue;
            };
            for k in 0..source.middles.len() {
                let Some(p) = source.middles.get(k).map(|q| crate::apply_point(m, *q)) else { continue };
                let near = |lo: f64, x: f64, hi: f64| lo - 1e-6 <= x && x <= hi + 1e-6;
                let (lo, hi) = (bounds.min, bounds.max);
                // A body the kernel can't classify against is not fitted to, rather than failing this frame.
                if !(near(lo.x, p.x, hi.x) && near(lo.y, p.y, hi.y) && near(lo.z, p.z, hi.z))
                    || !matches!(b.body.classify_point(p), Ok(PointClass::Inside))
                {
                    continue;
                }
                let at = crate::mat_mul(&to_f, m);
                let tool = match placed.entry((*g, k, bits(&at))) {
                    std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let base = source.tool(k)?;
                        let body = if crate::is_identity(&at) { base.clone() } else { kernel::transform_matrix(base, at)? };
                        e.insert(crate::lineage::numbered_tool(body)).clone()
                    }
                };
                out.push((b.name.clone(), tool));
                break 'body;
            }
        }
    }
    Ok(out)
}

/// A Move's rigid transform, `None` when its values don't evaluate.
fn move_matrix(vals: &BTreeMap<String, Value>, kind: &FeatureKind) -> Option<Mat> {
    let FeatureKind::Move { translate: [x, y, z], rotate_axis, origin, angle, .. } = kind else { return None };
    let len = |e: &str| val(vals, e, Kind::Length).ok();
    let t = Vec3::new(len(x)?, len(y)?, len(z)?);
    let angle = match angle {
        Some(a) => val(vals, a, Kind::Angle).ok()?,
        None => 0.0,
    };
    let origin = match origin {
        Some(p) => point(vals, p).ok()?,
        None => Vec3::ZERO,
    };
    Some(crate::rigid(t, origin, rotate_axis.map(Vec3::from).unwrap_or(Vec3::Z), angle))
}

/// A matrix's bits, to key it by.
fn bits(m: &Mat) -> [u64; 16] {
    let mut out = [0u64; 16];
    for (slot, v) in out.iter_mut().zip(m.iter().flatten()) {
        *slot = v.to_bits();
    }
    out
}

/// An earlier Frame Member's members, laid out and cut at their corners as [`super::member_section`] lays them, in
/// its own frame: a point in each one's material half way along it, and each one's tool once built.
struct Members {
    segments: Vec<(String, Vec3, Vec3)>,
    regions: Vec<Region2>,
    axes: Vec<Axes>,
    at: Vec2,
    ends: Vec<[Option<CutEnd>; 2]>,
    middles: Vec<Vec3>,
    tools: Vec<Option<kernel::Body>>,
}

impl Members {
    fn of(doc: &Document, vals: &BTreeMap<String, Value>, g: &Feature, st: &ModelState) -> Result<Members> {
        let FeatureKind::FrameMember { corner, corners, .. } = &g.kind else {
            return Err(DocError::Invalid("not a frame member feature".into()));
        };
        let Layout { segments, regions, at, turn, sketch_x, drawn, planes, .. } = layout(doc, vals, g, st)?;
        // ponytail: an earlier frame's corner that its own Fit Ends left Free is cut here as jointed (finding it
        // would build the tools of the frames before that one too); the tool then reaches on inside the tube that
        // corner lies in, which the fitted end is cut by anyway.
        let plan = auto_plan(*corner, &segments, &planes, corners, &drawn)?;
        let (axes, _) = section_axes(&segments, &planes, plan.as_ref(), turn)?;
        let inside = regions.first().map(|r| r.interior_point()).ok_or_else(|| DocError::Invalid("the member profile has no area".into()))?;
        let (ends, middles) = {
            let placed = placer(&axes, at);
            let ends = frame_ends(&g.kind, &segments, &regions, &placed, plan, (sketch_x, &drawn))?;
            (ends, segments.iter().enumerate().map(|(m, (_, a, b))| placed(m, (*a + *b) * 0.5).to_world(inside)).collect())
        };
        let tools = vec![None; segments.len()];
        Ok(Members { segments, regions, axes, at, ends, middles, tools })
    }

    /// Member m's tool, built the first time it is asked for.
    fn tool(&mut self, m: usize) -> Result<&kernel::Body> {
        let slot = self.tools.get_mut(m).ok_or_else(|| DocError::Invalid(format!("no member {m}")))?;
        if slot.is_none() {
            #[cfg(test)]
            TOOL_BUILDS.set(TOOL_BUILDS.get() + 1);
            let placed = placer(&self.axes, self.at);
            *slot = Some(member_tool(&self.segments, &self.regions, &placed, &self.ends, m)?.body);
        }
        slot.as_ref().ok_or_else(|| DocError::Invalid(format!("no member {m}")))
    }
}

#[cfg(test)]
#[path = "frame_earlier_tests.rs"]
mod tests;
