//! Frame members (#4003): one body per straight line of a frame sketch, and per pair of points
//! (#15705), its cross-section a profile laid square to the line (`section_axes`) and run
//! from one end to the other.

use solidcraft_geom::prelude::*;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::f64::consts::{PI, TAU};

use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::{Aabb2, Ring};
use solidcraft_kernel::{self as kernel, BoolOp};
use solidcraft_sketch::{CurveKind, Sketch};

use super::{ModelBody, ModelState, SolvedSketch, new_name, path_in_frame, select_profiles, sketch_in_feature_frame, val};
use crate::expr::{Kind, Value};
use crate::frame::{self, CornerOverride, CornerType, MAX_MEMBERS, MemberPair, MemberProfile};
use crate::frame_corners::{self, Corner, Cut};
use crate::frame_fit;
use crate::frame_layout::{self, ChainCorner, Fit, Joint, MemberPlane, Plan};
use crate::lineage::Traced;
use crate::{DocError, Document, Feature, FeatureKind, ProfileSel, Result};

#[path = "frame_earlier.rs"]
mod earlier;

/// Most profiles a profile sketch may have for its material to be worked out.
const MAX_PROFILE_PIECES: usize = 256;
/// Shortest member (mm).
const MIN_LENGTH: f64 = 1e-6;
/// How far (mm) a 3D sketch curve may stray from the line between its ends and still be a line.
const STRAIGHT_TOL: f64 = 1e-3;

/// Evaluate a `FrameMember` feature: one new body per line, each named
/// ([`member_names`], or by position for a copy whose `body_names` were set).
pub(super) fn evaluate(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &mut ModelState) -> Result<()> {
    let FeatureKind::FrameMember { body_name, names, corner, flip, corners, fit_ends, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let Layout { segments, regions, description, at, turn, sketch_x, drawn, planes } = layout(doc, vals, f, st)?;
    let mut plan = auto_plan(*corner, &segments, &planes, corners, &drawn)?;
    let (axes, plain) = section_axes(&segments, &planes, plan.as_ref(), turn)?;
    let (placed, unreversed) = (placer(&axes, at), unreversed_placer(&plain, at));
    // Fit Ends to Earlier Frames (None never fits): the earlier Frame Members' tubes and their copies, as cutting
    // tools, and this frame's own tubes laid before each free end (ruling A14), whether or not an earlier one exists.
    let fits = *fit_ends && !corner.is_none();
    let earlier = if fits { earlier::earlier_tools(doc, vals, f, st)? } else { Vec::new() };
    let centre = frame::anchor_point(&regions, frame::Anchor::Center);
    if let Some(plan) = plan.as_mut() {
        free_corners(plan, &earlier, &placed, centre);
    }
    // How far a member's tool may reach from its line: four times its section's farthest point from the line, plus
    // its corner extensions (its cut `ends`) and 1 mm.
    let section =
        regions.iter().flat_map(|r| &r.outer.segs).flat_map(|g| [g.start(), g.point_at(0.5), g.end()]).map(|q| q.dist(at)).fold(0.0, f64::max);
    let near_with = |ends: &[[Option<CutEnd>; 2]], k: usize, p: Vec3| {
        let ext = ends.get(k).map_or(0.0, |e| e.iter().flatten().map(|c| c.1).fold(0.0, f64::max));
        segments.get(k).is_some_and(|(_, a, b)| {
            let d = *b - *a;
            let t = ((p - *a).dot(d) / d.dot(d).max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
            p.dist(*a + d * t) <= 4.0 * section + ext + 1.0
        })
    };
    // This frame's own members' cut ends, to build their tools from, and each tool once built.
    let mut own_ends = if fits { frame_ends(&f.kind, &segments, &regions, &placed, plan.clone(), (sketch_x, &drawn))? } else { Vec::new() };
    let mut tools: Vec<Option<kernel::Body>> = vec![None; segments.len()];
    if fits && let Some(plan) = plan.as_mut() {
        // A chain corner that turns Free changes only its chain's members' ends, never a group member's (whose tools
        // it builds): the tools built stay right.
        let tool = |k: usize| member_tool(&segments, &regions, &placed, &own_ends, k);
        if free_chains(plan, &earlier, (&placed, centre), &|k, p| near_with(&own_ends, k, p), &tool, &mut tools, &segments)? {
            own_ends = frame_ends(&f.kind, &segments, &regions, &placed, Some(plan.clone()), (sketch_x, &drawn))?;
        }
    }
    let unjointed = if fits { unjointed_ends(&f.kind, &segments, plan.as_ref()) } else { Vec::new() };
    // This frame's own members, for its free ends: their laying order (Auto's, else the drawing order, never the
    // pick order), and their cut ends to build their tools from.
    let mut laid = vec![usize::MAX; segments.len()];
    let order: Vec<usize> = match plan.as_ref() {
        Some(plan) => plan.ranks.iter().map(|r| r.member).collect(),
        None => {
            let mut order: Vec<usize> = (0..segments.len()).collect();
            order.sort_by_key(|&m| (drawn.get(m).copied().unwrap_or(m), m));
            order
        }
    };
    for (i, m) in order.into_iter().enumerate() {
        if let Some(p) = laid.get_mut(m) {
            *p = i;
        }
    }
    // Only an Auto frame builds by Auto's rules; an Auto override elsewhere is the frame's own type (corner_ends).
    let mut cut = if let Some(plan) = plan {
        auto_members(&segments, &regions, (&placed, &unreversed), plan, (sketch_x, &drawn))?
    } else if corner.is_none() && corners.is_empty() {
        // A frame with no corners to finish is built exactly as it always was.
        Vec::new()
    } else {
        let (ends, copes) = corner_ends(&segments, &regions, &placed, (*corner, *flip), corners, (sketch_x, &drawn))?;
        cut_members(&segments, &regions, (&placed, &|_, _| None), &ends, copes)?
    };
    let mut made = Vec::with_capacity(segments.len());
    for (m, (id, a, b)) in segments.iter().enumerate() {
        if let Some(t) = cut.get_mut(m).and_then(Option::take) {
            made.push(t);
            continue;
        }
        let (x, y) = axes.get(m).copied().ok_or_else(|| DocError::Invalid(format!("line `{id}` has no length")))?;
        let plane = Plane::new(*a - x * at.x - y * at.y, x, y).ok_or_else(|| DocError::Invalid(format!("line `{id}`: no plane square to it")))?;
        made.push(member_body(plane, unreversed(m, *a), &regions, *b - *a, (0.0, a.dist(*b)), id)?);
    }
    // A point in the section's material (a tube's wall, not its bore), half way along member m.
    let inside = regions.first().map(|r| r.interior_point()).ok_or_else(|| DocError::Invalid("the member profile has no area".into()))?;
    let middle = |m: usize| segments.get(m).map_or(Vec3::ZERO, |(_, a, b)| placed(m, (*a + *b) * 0.5).to_world(inside));
    let near = |k: usize, p: Vec3| near_with(&own_ends, k, p);
    let tool = |k: usize| member_tool(&segments, &regions, &placed, &own_ends, k);
    fit_to_earlier(&mut made, &segments, &unjointed, &earlier, &mut Own { laid, near: &near, tool: &tool, tools }, &middle)?;
    if !f.body_names.is_empty() {
        // A copy (pasted, or in a copied component) names its members by position, as any
        // feature's bodies.
        for (k, t) in made.into_iter().enumerate() {
            let name = new_name(st, f, k);
            st.bodies.push(ModelBody::traced_new(name, t, f.id));
        }
        return Ok(());
    }
    let base = body_name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map_or(description, str::to_string);
    let ids: Vec<String> = segments.into_iter().map(|(id, _, _)| id).collect();
    for (name, t) in member_names(st, &ids, names, &base).into_iter().zip(made) {
        st.bodies.push(ModelBody::traced_new(name, t, f.id));
    }
    Ok(())
}

/// The one body of an extruded member.
fn one_solid(mut bodies: Vec<kernel::Body>, id: &str) -> Result<kernel::Body> {
    match (bodies.pop(), bodies.is_empty()) {
        (Some(body), true) => Ok(body),
        _ => Err(DocError::Invalid(format!("the member on line `{id}` is not one solid"))),
    }
}

/// A member's cut end: its plane cut (`None`: a coped end, built out but not cut by a plane), how
/// far past the end it is built, and whether an End Cap may go on it (a through member's end of
/// a Butt or Coped corner, cut square to its own line).
type CutEnd = (Option<Cut>, f64, bool);

/// Each member's cut at its [start, end] and each Coped corner's (T, U, corner point, n). Shared
/// by [`cut_members`] and [`member_section`], so both see the same corners.
type CornerEnds = (Vec<[Option<CutEnd>; 2]>, Vec<(usize, usize, Vec3, Vec3)>);

/// What breaks a tie for which member runs through: the sketch's X axis (`None`: a 3D sketch) and
/// each member's place in the sketch's drawing order.
type Tie<'a> = (Option<Vec3>, &'a [usize]);

fn corner_ends(
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    placed: &dyn Fn(usize, Vec3) -> Plane,
    finish: (CornerType, bool),
    overrides: &[CornerOverride],
    (sketch_x, drawn): Tie,
) -> Result<CornerEnds> {
    // A design file is hostile input: the command checks the list, a file may not.
    if overrides.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} corner overrides in one feature")));
    }
    fn pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
        if a <= b { (a, b) } else { (b, a) }
    }
    let id = |m: usize| segments.get(m).map_or("", |s| s.0.as_str());
    let corners = frame_corners::find_corners(segments);
    let at: HashMap<(&str, &str), usize> = corners
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let [(a, _), (b, _)] = c.ends;
            (pair(id(a), id(b)), k)
        })
        .collect();
    let mut finishes = vec![finish; corners.len()];
    for o in overrides {
        // Two members: the corner they meet at. Three or more are an Auto frame's alone (which never comes here).
        let [a, b] = o.members.as_slice() else {
            if o.members.len() < 2 {
                return Err(DocError::Invalid("A corner override names at least two members".into()));
            }
            return Err(DocError::Invalid("A corner override is not at a corner of the frame".into()));
        };
        let k = at.get(&pair(a, b)).ok_or_else(|| DocError::Invalid("A corner override is not at a corner of the frame".into()))?;
        if let Some(f) = finishes.get_mut(*k) {
            // An Auto override on a frame that isn't Auto is the frame's own type, with its own Flip.
            *f = (if o.corner == CornerType::Auto { finish.0 } else { o.corner }, o.flip);
        }
    }
    // Each member's cut ends, and each Coped Butt's (T, U): U loses T's filled shape once every plane cut is made.
    let mut out: CornerEnds = (vec![[None, None]; segments.len()], Vec::new());
    for (c, (kind, flip)) in corners.iter().zip(finishes) {
        let cuts = frame_corners::corner_cuts(kind, flip, c, segments, placed, regions, sketch_x, drawn)?;
        add_corner(&mut out, c, kind.flips(), cuts, segments, regions, placed)?;
    }
    Ok(out)
}

/// A corner's cuts as [`frame_corners::corner_cuts`] gives them, put into `ends` (each cut end with how far past
/// its line's end it is built) and `copes`. `butt`: the first cut is a butt's T's, square to its line, which must not
/// run back past U's far end, and is the one cut end an End Cap may go on.
fn add_corner(
    (ends, copes): &mut CornerEnds,
    c: &Corner,
    butt: bool,
    (cuts, cope): (Vec<(usize, bool, Cut)>, Option<(usize, usize)>),
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    placed: &dyn Fn(usize, Vec3) -> Plane,
) -> Result<()> {
    let id = |m: usize| segments.get(m).map_or("", |s| s.0.as_str());
    for (k, (m, at_start, cut)) in cuts.into_iter().enumerate() {
        let away = frame_corners::away(segments, (m, at_start));
        let ext = away
            .and_then(|w| frame_corners::extension(&placed(m, c.point), regions, c.point, w, &cut))
            .ok_or_else(|| DocError::Invalid(format!("the member on line `{}` can't be built out to its corner cut", id(m))))?;
        // A butt's first cut is T's. T must not run back past the far end of U, the corner's other member.
        if k == 0 && butt {
            let [(e0, _), (e1, _)] = c.ends;
            let u = if e0 == m { e1 } else { e0 };
            let u_len = segments.get(u).map_or(0.0, |(_, a, b)| a.dist(*b));
            let u_reach = away.map_or(0.0, |w| frame_corners::reach(&placed(u, c.point), regions, c.point, -w));
            if ext - 1.0 > u_len + u_reach {
                return Err(frame_corners::corner_error(c.point, "too flat to butt"));
            }
        }
        // A butt's first cut is T's, square to its line: the one cut end a cap may go on.
        let cap = k == 0 && butt;
        if let Some(end) = ends.get_mut(m).and_then(|e| e.get_mut(usize::from(!at_start))) {
            *end = Some((Some(cut), ext, cap));
        }
    }
    let Some((t, u)) = cope else { return Ok(()) };
    // U is built out across the whole of T, to past T's far side: the cope trims it back.
    let (te, ue) = (frame_corners::end_at(c, t)?, frame_corners::end_at(c, u)?);
    // n: square to T, towards U's side of it; T's near face lies `near` along it.
    let reached = match (frame_corners::away(segments, te), frame_corners::away(segments, ue)) {
        (Some(wt), Some(wu)) => (wu - wt * wu.dot(wt)).normalized().and_then(|n| {
            let far = Cut { origin: c.point - n * frame_corners::reach(&placed(t, c.point), regions, c.point, -n), normal: n };
            let near = frame_corners::reach(&placed(t, c.point), regions, c.point, n);
            frame_corners::extension(&placed(u, c.point), regions, c.point, wu, &far).map(|ext| (n, near, ext))
        }),
        _ => None,
    };
    let (n, near, ext) = reached.ok_or_else(|| DocError::Invalid(format!("the member on line `{}` can't be built out to its corner", id(u))))?;
    // U wholly inside T (its far end short of T's near face) keeps nothing of its own.
    let far_end = segments.get(u).map(|(_, a, b)| if ue.1 { *b } else { *a });
    if far_end.is_none_or(|q| (q - c.point).dot(n) <= near) {
        return Err(DocError::Invalid(format!("the member on line `{}` was cut away completely", id(u))));
    }
    if let Some(end) = ends.get_mut(u).and_then(|e| e.get_mut(usize::from(!ue.1))) {
        *end = Some((None, ext, false));
    }
    copes.push((t, u, c.point, n));
    Ok(())
}

/// Each member cut at its corners (#4003) as `ends` and `copes` say ([`corner_ends`], [`auto_ends`]), or `None` for
/// a member with no cut end (built square, as without corners). A cut member is built past each cut end far enough
/// to reach the whole cut, then cut there; the cut face is named `corner_start` or `corner_end`. A Coped Butt's U is
/// built out across T and, after every plane cut, loses T's filled shape, keeping its largest piece. `placed(m, p)`
/// is member m's section plane, square to its line, placed at `p`.
fn cut_members(
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    (placed, unreversed): Placing,
    ends: &[[Option<CutEnd>; 2]],
    copes: Vec<(usize, usize, Vec3, Vec3)>,
) -> Result<Vec<Option<Traced>>> {
    let id = |m: usize| segments.get(m).map_or("", |s| s.0.as_str());
    // Member m built out at its cut ends and cut there, from `regions`.
    let build = |m: usize, regions: &[Region2]| -> Result<Traced> {
        let ((id, a, b), [start, end]) = segments.get(m).zip(ends.get(m)).ok_or_else(|| DocError::Invalid(format!("no member {m}")))?;
        let (lo, hi) = (start.map_or(0.0, |c| c.1), end.map_or(0.0, |c| c.1));
        let mut t = member_body(placed(m, *a), unreversed(m, *a), regions, *b - *a, (-lo, a.dist(*b) + hi), id)?;
        for (cut, role) in [(start, "corner_start"), (end, "corner_end")] {
            let Some((Some(cut), _, _)) = cut else { continue };
            let plane =
                Plane::from_normal(cut.origin, cut.normal).ok_or_else(|| DocError::Invalid(format!("line `{id}`: a corner cut has no plane")))?;
            let (part, tool) = kernel::half_space(&t.body, &plane, true)?;
            let part = part.ok_or_else(|| DocError::Invalid(format!("the member on line `{id}` was cut away completely")))?;
            // The cut face is the cap of the half-space box in the plane: its start cap.
            let tool = crate::lineage::ProfileRoles { sketch: None, region: None, caps: [Some(role.into()), None], thin: None }.tool(tool, None);
            t = Traced::step(part, &[&t, &tool]);
        }
        Ok(t)
    };
    let mut out = Vec::with_capacity(segments.len());
    for (m, (id, _, _)) in segments.iter().enumerate() {
        if ends.get(m).is_none_or(|e| e.iter().all(Option::is_none)) {
            out.push(None);
            continue;
        }
        let t = build(m, regions)?;
        if t.body.lumps()?.len() > 1 {
            return Err(DocError::Invalid(format!("the member on line `{id}` is cut into pieces by its corners")));
        }
        out.push(Some(t));
    }
    // Copes after every plane cut. T filled is T built from its outlines alone, holes closed;
    // U keeps its largest piece on its own side of T, its centre of mass on U's side (what lies
    // past T, and slivers on T's far side, go).
    let outlines: Vec<Region2> = regions.iter().map(|r| Region2 { outer: r.outer.clone(), holes: Vec::new() }).collect();
    for (t, u, point, n) in copes {
        let filled = crate::lineage::numbered_tool(build(t, &outlines)?.body);
        let gone = || DocError::Invalid(format!("the member on line `{}` was cut away completely", id(u)));
        let traced = out.get_mut(u).and_then(Option::as_mut).ok_or_else(gone)?;
        let cut = kernel::boolean(&traced.body, &filled.body, BoolOp::Cut)?.ok_or_else(gone)?;
        let cut = Traced::step(cut, &[traced, &filled]);
        let lumps = cut.body.lumps()?;
        let lineages = crate::lineage::Lineage::lumps(&cut.body, cut.lineage.as_ref(), &lumps);
        let mut best: Option<(f64, Traced)> = None;
        for (body, lineage) in lumps.into_iter().zip(lineages) {
            let mass = kernel::measure_mass(&body)?;
            let v = mass.volume;
            if (mass.centroid - point).dot(n) > 0.0 && best.as_ref().is_none_or(|(most, _)| v > *most) {
                best = Some((v, Traced { body, lineage, recipe: None }));
            }
        }
        *traced = best.ok_or_else(gone)?.1;
    }
    Ok(out)
}

/// An Auto frame's plan ([`frame_layout::plan`]); `None` for any other corner type, as only an Auto frame builds by
/// Auto's rules and lines its sections up.
fn auto_plan(
    corner: CornerType,
    segments: &[(String, Vec3, Vec3)],
    planes: &[MemberPlane],
    overrides: &[CornerOverride],
    drawn: &[usize],
) -> Result<Option<Plan>> {
    if corner != CornerType::Auto {
        return Ok(None);
    }
    // A design file is hostile input: the command checks the list, a file may not.
    if overrides.len() > MAX_MEMBERS || overrides.iter().any(|o| o.members.len() > MAX_MEMBERS) {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} corner overrides of at most {MAX_MEMBERS} members in one feature")));
    }
    if overrides.iter().any(|o| o.members.len() < 2) {
        return Err(DocError::Invalid("A corner override names at least two members".into()));
    }
    // One feature's members share one section.
    frame_layout::plan(segments, planes, drawn, &|_, _| true, overrides).map(Some)
}

/// Each member's section axes (x, y), decided here alone so that its build, its corner cuts, its fit tool and End Cap
/// ([`member_section`]) all place it alike. Under Auto (`plan`) sections line up (#4003 Goal 7): a member in a sketch
/// plane takes the sketch's normal as up and runs the way its group lays it, an outer member round the outline
/// ([`frame_layout::Rank`]); members in no sketch plane chained in one plane take that plane's normal, pointing up
/// ([`chain_ups`]). Any other member, and every member of a frame that isn't Auto, keeps [`frame::member_axes`].
/// Also, for each member Auto runs against its line, its axes running the way the line is drawn, to name its faces
/// by ([`member_body`]).
fn section_axes(segments: &[(String, Vec3, Vec3)], planes: &[MemberPlane], plan: Option<&Plan>, turn: f64) -> Result<(Vec<Axes>, Vec<Option<Axes>>)> {
    let mut ups: Vec<Option<Vec3>> = vec![None; segments.len()];
    let mut reverse = vec![false; segments.len()];
    if let Some(plan) = plan {
        ups = chain_ups(segments, &plan.chained);
        for (up, plane) in ups.iter_mut().zip(planes) {
            if let Some((normal, _)) = plane {
                *up = Some(*normal);
            }
        }
        for r in &plan.ranks {
            if let Some(v) = reverse.get_mut(r.member) {
                *v = r.reverse;
            }
        }
    }
    let (mut axes, mut plain) = (Vec::with_capacity(segments.len()), Vec::with_capacity(segments.len()));
    for (m, (id, a, b)) in segments.iter().enumerate() {
        let reversed = reverse.get(m).copied().unwrap_or(false);
        let up = ups.get(m).copied().flatten();
        let lined = up.and_then(|up| frame::group_axes(if reversed { *a - *b } else { *b - *a }, up, turn));
        plain.push(up.filter(|_| reversed && lined.is_some()).and_then(|up| frame::group_axes(*b - *a, up, turn)));
        axes.push(lined.or_else(|| frame::member_axes(*b - *a, turn)).ok_or_else(|| DocError::Invalid(format!("line `{id}` has no length")))?);
    }
    Ok((axes, plain))
}

/// A section's (x, y) in the world.
type Axes = (Vec3, Vec3);

/// Each member's section plane placed on its line ([`placer`]), and for a member Auto runs against its line, its
/// plane as it would be placed running the way its line is drawn ([`unreversed_placer`]).
type Placing<'a> = (&'a dyn Fn(usize, Vec3) -> Plane, &'a dyn Fn(usize, Vec3) -> Option<Plane>);

/// Each member's up when it is in a chain of members in no sketch plane (joined at `chained`'s corners) that lies in
/// one plane: that plane's normal, pointing up ([`plane_up`]). `None` for any other member.
fn chain_ups(segments: &[(String, Vec3, Vec3)], chained: &[ChainCorner]) -> Vec<Option<Vec3>> {
    // Each member's chain, as the lowest member in it: joining two chains points the higher one at the lower, so a
    // walk down `chain` always ends.
    let mut chain: Vec<usize> = (0..segments.len()).collect();
    let root = |chain: &[usize], mut m: usize| {
        while let Some(&p) = chain.get(m).filter(|&&p| p < m) {
            m = p;
        }
        m
    };
    for c in chained {
        let (ra, rb) = (root(&chain, c.a.0), root(&chain, c.b.0));
        if let Some(slot) = chain.get_mut(ra.max(rb)) {
            *slot = ra.min(rb);
        }
    }
    let mut chains: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for m in 0..segments.len() {
        chains.entry(root(&chain, m)).or_default().push(m);
    }
    let mut out = vec![None; segments.len()];
    for ms in chains.values().filter(|ms| ms.len() > 1) {
        let points: Vec<Vec3> = ms.iter().filter_map(|&m| segments.get(m)).flat_map(|(_, a, b)| [*a, *b]).collect();
        let up = plane_up(&points);
        for &m in ms {
            if let Some(o) = out.get_mut(m) {
                *o = up;
            }
        }
    }
    out
}

/// The unit normal of the one plane `points` lie in (within CORNER_TOL), pointing up: +Z, else +Y, else +X
/// (claude_fusion's `chain_up`). `None` when they lie along one line, or in no one plane.
fn plane_up(points: &[Vec3]) -> Option<Vec3> {
    let tol = frame_corners::CORNER_TOL;
    let p0 = *points.first()?;
    let v1 = points.iter().map(|p| *p - p0).find(|v| v.len() > tol)?;
    let n = points.iter().map(|p| v1.cross(*p - p0)).find(|c| c.len() > tol * v1.len())?.normalized()?;
    // `!(… <= tol)`, so a point that isn't finite is in no plane.
    if points.iter().any(|p| !((*p - p0).dot(n).abs() <= tol)) {
        return None;
    }
    let first = [n.z, n.y, n.x].into_iter().find(|c| c.abs() > 1e-9).unwrap_or(1.0);
    Some(if first < 0.0 { n * -1.0 } else { n })
}

/// Member m's section plane, square to its line, placed at `p` on it: `axes` from [`section_axes`], `at` the profile
/// point that sits on the line. A member Auto runs against its line has x × y against it too ([`along`]).
fn placer(axes: &[Axes], at: Vec2) -> impl Fn(usize, Vec3) -> Plane + '_ {
    move |m, p| section_plane(axes.get(m).copied().unwrap_or((Vec3::X, Vec3::Y)), at, p)
}

/// [`placer`] for the members [`section_axes`] gives unreversed axes: `None` for every other member.
fn unreversed_placer(plain: &[Option<Axes>], at: Vec2) -> impl Fn(usize, Vec3) -> Option<Plane> + '_ {
    move |m, p| plain.get(m).copied().flatten().map(|xy| section_plane(xy, at, p))
}

/// The section plane of axes (x, y) whose profile point `at` sits at `p`.
fn section_plane((x, y): Axes, at: Vec2, p: Vec3) -> Plane {
    let origin = p - x * at.x - y * at.y;
    Plane::new(origin, x, y).unwrap_or(Plane { origin, x, y })
}

/// A member's body: `regions` on its section `plane` at its start, built along its line `d` ([`along`]) from `lo` to
/// `hi`, each face named `face:<k>`. A member Auto runs against its line is built from its regions mirrored, which
/// the kernel numbers otherwise, so its faces are named as the same member built on `unreversed` (its plane running
/// the way its line is drawn) numbers them ([`matched_numbers`]), each cap as the cap at the same end:
/// - each side face as the face at the same place in the section, measured from the centre of the section's box: a
///   section symmetric about its own centre line is then the same solid either way, or moved across its line, and
///   each face keeps its name;
/// - failing that (a section that is not, such as an angle or a channel), as the face swept from the same edge of
///   the section;
/// - failing both, as the kernel numbers it.
///
/// The kernel numbers an extrusion's faces by where it lies, not by profile segment, so they can't be renumbered by
/// index alone.
fn member_body(plane: Plane, unreversed: Option<Plane>, regions: &[Region2], d: Vec3, (lo, hi): (f64, f64), id: &str) -> Result<Traced> {
    let (plane, built) = along(plane, regions, d);
    let body = one_solid(kernel::extrude(&plane, &built, lo, hi)?, id)?;
    let (Cow::Owned(_), Some(unreversed)) = (&built, unreversed) else {
        return Ok(crate::lineage::numbered_tool(body));
    };
    let plain = one_solid(kernel::extrude(&unreversed, regions, lo, hi)?, id)?;
    let (mid_m, mid_p) = (frame::anchor_point(&built, frame::Anchor::Center), frame::anchor_point(regions, frame::Anchor::Center));
    let at_place = matched_numbers(&body, &plain, |q| plane.dir_to_world(q - mid_m), |q| unreversed.dir_to_world(q - mid_p));
    let by_edge = || matched_numbers(&body, &plain, |q| Vec3::new(-q.x, q.y, 0.0), |q| Vec3::new(q.x, q.y, 0.0));
    Ok(match at_place.or_else(by_edge) {
        Some(k) => Traced::made(body, Some(k.into_iter().map(|k| vec![format!("face:{k}")]).collect())),
        None => crate::lineage::numbered_tool(body),
    })
}

/// For each face of `mirrored` (a section's regions mirrored and extruded), the face of `plain` (the same section
/// extruded unmirrored) whose profile segment `fp` maps where `fm` maps the mirrored face's (both ends and its
/// middle), or the cap at the same end. `None` unless each face finds its own.
fn matched_numbers(mirrored: &kernel::Body, plain: &kernel::Body, fm: impl Fn(Vec2) -> Vec3, fp: impl Fn(Vec2) -> Vec3) -> Option<Vec<usize>> {
    use kernel::FaceSource::{ExtrudeCap, ExtrudeSide};
    fn source(b: &kernel::Body, i: usize) -> Option<&kernel::FaceSource> {
        b.face_sources().and_then(|s| s.get(i)).and_then(Option::as_ref)
    }
    let near = |p: Vec2, q: Vec2| fm(p).dist(fp(q)) <= 1e-6;
    let same = |i: usize, j: usize| match (source(mirrored, i), source(plain, j)) {
        (Some(ExtrudeCap { high: a, .. }), Some(ExtrudeCap { high: b, .. })) => a == b,
        (Some(ExtrudeSide { segment: s, .. }), Some(ExtrudeSide { segment: t, .. })) => {
            let (a, b) = (s.start(), s.end());
            near(s.point_at(0.5), t.point_at(0.5)) && ((near(a, t.start()) && near(b, t.end())) || (near(a, t.end()) && near(b, t.start())))
        }
        _ => false,
    };
    let n = plain.face_count();
    if mirrored.face_count() != n {
        return None;
    }
    let mut taken = vec![false; n];
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let j = (0..n).find(|&j| taken.get(j) == Some(&false) && same(i, j))?;
        if let Some(t) = taken.get_mut(j) {
            *t = true;
        }
        out.push(j);
    }
    Some(out)
}

/// A member's section `plane` and `regions` to build along its line `d`. A member Auto runs against its line
/// ([`section_axes`]) is placed with x × y against the line: here its x is turned round and its regions mirrored, the
/// same section in a plane whose x × y runs along the line, so every member is built from its line's drawn start, its
/// ends and their names tied to its drawn ends. Any other plane is given back as it is.
fn along(plane: Plane, regions: &[Region2], d: Vec3) -> (Plane, Cow<'_, [Region2]>) {
    if plane.normal().dot(d) >= 0.0 {
        return (plane, Cow::Borrowed(regions));
    }
    (Plane { x: plane.x * -1.0, ..plane }, Cow::Owned(regions.iter().map(frame::mirrored).collect()))
}

/// Auto's ends (#4003 spec A4-A6), as [`corner_ends`] gives stage 1's, and the ends cut to fit, in laying order:
/// [`frame_layout::plan`]'s corners jointed by the table (an angled butt, a mitre, or a butt whose short tube is
/// fitted to the long one) or by their override; its chained corners as stage 1 finishes them; its fits. A fitted
/// end is built to its line's end and not plane cut, as `(None, 0, false)`.
fn auto_ends(
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    placed: &dyn Fn(usize, Vec3) -> Plane,
    plan: Plan,
    (sketch_x, drawn): Tie,
) -> Result<(CornerEnds, Vec<Fit>)> {
    let mut out: CornerEnds = (vec![[None, None]; segments.len()], Vec::new());
    let mut fits = plan.fits;
    for ac in &plan.corners {
        let c = Corner { point: ac.point, ends: [ac.long, ac.short] };
        let inside = frame_corners::interior(segments, &c);
        // In line (the table's angled butt at 0°): both left square, whatever the joint. A Free corner is not
        // jointed here either.
        if inside > PI - frame_corners::IN_LINE_DEG.to_radians() || (ac.kind, ac.joint) == (CornerType::Auto, Joint::Free) {
            continue;
        }
        if ac.kind != CornerType::None && inside < frame_corners::MIN_INTERIOR_DEG.to_radians() {
            return Err(frame_corners::corner_error(c.point, "too sharp to finish"));
        }
        // corner_cuts runs a butt's longer member through, then Flip swaps it; `long` already is the one to run
        // through (the plan applied Flip), so ask for whichever Flip keeps it.
        let flip = frame_corners::through(segments, &c, sketch_x, drawn) != 0;
        let cuts = |kind| frame_corners::corner_cuts(kind, flip, &c, segments, placed, regions, sketch_x, drawn);
        match (ac.kind, ac.joint) {
            (CornerType::Auto, Joint::AngledButt) => {
                let angled = frame_corners::angled_butt_cuts(&c, ac.long.0, ac.short.0, segments);
                add_corner(&mut out, &c, false, (angled, None), segments, regions, placed)?;
            }
            (CornerType::Auto, Joint::Mitre) => add_corner(&mut out, &c, false, cuts(CornerType::Mitre)?, segments, regions, placed)?,
            (CornerType::Auto, _) => {
                // A butt: the long tube runs through, cut as stage 1's T; the short one is cut to fit against it.
                let (mut through, _) = cuts(CornerType::Butt)?;
                through.truncate(1);
                add_corner(&mut out, &c, true, (through, None), segments, regions, placed)?;
                fits.push(Fit { member: ac.short.0, at_start: ac.short.1, against: vec![ac.long.0] });
            }
            (kind, _) => add_corner(&mut out, &c, kind.flips(), cuts(kind)?, segments, regions, placed)?,
        }
    }
    // Members in no group plane, chained: mitred, or as their override says, by stage 1's rules.
    let end = |(m, at_start): (usize, bool)| segments.get(m).map(|(_, a, b)| if at_start { *a } else { *b });
    for ch in &plan.chained {
        let (Some(pa), Some(pb)) = (end(ch.a), end(ch.b)) else { continue };
        let c = Corner { point: (pa + pb) * 0.5, ends: [ch.a, ch.b] };
        let cuts = frame_corners::corner_cuts(ch.kind, ch.flip, &c, segments, placed, regions, sketch_x, drawn)?;
        add_corner(&mut out, &c, ch.kind.flips(), cuts, segments, regions, placed)?;
    }
    let mut place = vec![usize::MAX; segments.len()];
    for (i, r) in plan.ranks.iter().enumerate() {
        if let Some(p) = place.get_mut(r.member) {
            *p = i;
        }
    }
    fits.sort_by_key(|f| place.get(f.member).copied().unwrap_or(usize::MAX));
    for f in &fits {
        if let Some(e) = out.0.get_mut(f.member).and_then(|e| e.get_mut(usize::from(!f.at_start)))
            && e.is_none()
        {
            *e = Some((None, 0.0, false));
        }
    }
    Ok((out, fits))
}

/// An Auto frame's members (#4003 stage 2): cut at their corners ([`auto_ends`], [`cut_members`]), then each end to
/// fit cut, in laying order, to the members it meets as their plane cuts build them (each member's cutting tool,
/// [`frame_fit::fit_tool`], made once). A fitted face is named for the end it finishes and the line of the member it
/// fits, `corner_start:<line>:face:<k>` (`corner_end:…`), so a member's fitted faces keep names of their own whatever
/// the laying or pick order. `plan`: the frame's [`frame_layout::plan`].
pub(crate) fn auto_members(
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    (placed, unreversed): Placing,
    plan: Plan,
    tie: Tie,
) -> Result<Vec<Option<Traced>>> {
    let ((ends, copes), fits) = auto_ends(segments, regions, placed, plan, tie)?;
    let mut out = cut_members(segments, regions, (placed, unreversed), &ends, copes)?;
    let no_member = |m: usize| DocError::Invalid(format!("no member {m}"));
    let mut tools: Vec<Option<kernel::Body>> = (0..segments.len()).map(|_| None).collect();
    for &t in fits.iter().flat_map(|f| &f.against) {
        if let Some(slot) = tools.get_mut(t)
            && slot.is_none()
        {
            *slot = Some(member_tool(segments, regions, placed, &ends, t)?.body);
        }
    }
    // A point in the section's material (a tube's wall, not its bore).
    let inside = regions.first().map(|r| r.interior_point()).ok_or_else(|| DocError::Invalid("the member profile has no area".into()))?;
    for f in &fits {
        let (id, a, b) = segments.get(f.member).ok_or_else(|| no_member(f.member))?;
        let (end, toward) = if f.at_start { (*a, *a - *b) } else { (*b, *b - *a) };
        let middle = placed(f.member, (*a + *b) * 0.5).to_world(inside);
        let end_role = if f.at_start { "corner_start" } else { "corner_end" };
        let named: Vec<Traced> = f
            .against
            .iter()
            .filter_map(|&t| {
                let (body, line) = (tools.get(t)?.as_ref()?, &segments.get(t)?.0);
                let roles = (0..body.face_count()).map(|k| vec![format!("{end_role}:{line}:face:{k}")]).collect();
                Some(Traced::made(body.clone(), Some(roles)))
            })
            .collect();
        let against: Vec<&Traced> = named.iter().collect();
        let slot = out.get_mut(f.member).ok_or_else(|| no_member(f.member))?;
        let member = slot.take().ok_or_else(|| no_member(f.member))?;
        *slot = Some(frame_fit::fit_end(&member, end, toward, middle, &against, id)?);
    }
    Ok(out)
}

/// Member m's cutting tool ([`frame_fit::fit_tool`]) as its plane cuts (`ends`) build it: a fitted or coped end stops
/// at its line's end, inside the tube it meets.
fn member_tool(
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    placed: &dyn Fn(usize, Vec3) -> Plane,
    ends: &[[Option<CutEnd>; 2]],
    m: usize,
) -> Result<Traced> {
    let (_, a, b) = segments.get(m).ok_or_else(|| DocError::Invalid(format!("no member {m}")))?;
    // No ends listed: a frame with no corners to finish, its members square.
    let [start, end] = ends.get(m).copied().unwrap_or([None, None]);
    let cut = |e: Option<CutEnd>| e.and_then(|(cut, ext, _)| Some((cut?, ext)));
    let (start, end) = (cut(start), cut(end));
    let cuts: Vec<Cut> = [start, end].into_iter().flatten().map(|c| c.0).collect();
    let (plane, regions) = along(placed(m, *a), regions, *b - *a);
    frame_fit::fit_tool(&plane, &regions, -start.map_or(0.0, |c| c.1), a.dist(*b) + end.map_or(0.0, |c| c.1), &cuts)
}

/// Each member's cut ends as the feature's corners make them ([`auto_ends`] for an Auto frame, else [`corner_ends`];
/// none for a frame with no corners to finish).
fn frame_ends(
    f: &FeatureKind,
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    placed: &dyn Fn(usize, Vec3) -> Plane,
    plan: Option<Plan>,
    tie: Tie,
) -> Result<Vec<[Option<CutEnd>; 2]>> {
    let FeatureKind::FrameMember { corner, flip, corners, .. } = f else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    Ok(if let Some(plan) = plan {
        auto_ends(segments, regions, placed, plan, tie)?.0.0
    } else if corner.is_none() && corners.is_empty() {
        Vec::new()
    } else {
        corner_ends(segments, regions, placed, (*corner, *flip), corners, tie)?.0
    })
}

/// An Auto frame's corners that lie inside the earlier Frame Members' tubes (`earlier`, [`earlier::earlier_tools`]):
/// the long tube's section centre there [`holds`] in them. Each becomes [`Joint::Free`], not jointed, unless an
/// override names it (#4003 decision 1).
fn free_corners(plan: &mut Plan, earlier: &[(String, Traced)], placed: &dyn Fn(usize, Vec3) -> Plane, centre: Vec2) {
    if earlier.is_empty() {
        return;
    }
    for c in plan.corners.iter_mut().filter(|c| c.kind == CornerType::Auto && !c.overridden) {
        if holds(placed(c.long.0, c.point).to_world(centre), earlier.iter().map(|(_, t)| &t.body)) {
            c.joint = Joint::Free;
        }
    }
}

/// An Auto frame's chain corners (between members in no sketch plane, [`frame_layout::ChainCorner`]) that no override
/// names and whose section centre [`holds`] in a tube, an earlier Frame Member's (`earlier`) or one of this frame's
/// own group members' as built (`tool`, each built once into `tools`, only for members `near` the point): taken out
/// of `plan.chained`, so neither end is jointed there and both are free ends, fitted with the others (ruling A15;
/// claude_fusion `_chain_members`). `placed` and `centre` place the section's centre. Whether any was.
fn free_chains(
    plan: &mut Plan,
    earlier: &[(String, Traced)],
    (placed, centre): (&dyn Fn(usize, Vec3) -> Plane, Vec2),
    near: &dyn Fn(usize, Vec3) -> bool,
    tool: &dyn Fn(usize) -> Result<Traced>,
    tools: &mut [Option<kernel::Body>],
    segments: &[(String, Vec3, Vec3)],
) -> Result<bool> {
    // The members in no group plane share the group after the last one.
    let rank = |m: usize| plan.ranks.iter().find(|r| r.member == m);
    let Some(rest) = plan.chained.first().and_then(|c| rank(c.a.0)).map(|r| r.group) else { return Ok(false) };
    let grouped: Vec<usize> = plan.ranks.iter().filter(|r| r.group < rest).map(|r| r.member).collect();
    let end = |(m, at_start): (usize, bool)| segments.get(m).map(|(_, a, b)| if at_start { *a } else { *b });
    let before = plan.chained.len();
    let mut kept = Vec::with_capacity(before);
    for c in std::mem::take(&mut plan.chained) {
        let (Some(pa), Some(pb), false) = (end(c.a), end(c.b), c.overridden) else {
            kept.push(c);
            continue;
        };
        let p = placed(c.a.0, (pa + pb) * 0.5).to_world(centre);
        let mine: Vec<usize> = grouped.iter().copied().filter(|&k| near(k, p)).collect();
        for &k in &mine {
            if let Some(slot) = tools.get_mut(k)
                && slot.is_none()
            {
                *slot = Some(tool(k)?.body);
            }
        }
        let own = mine.iter().filter_map(|&k| tools.get(k)?.as_ref());
        if !holds(p, earlier.iter().map(|(_, t)| &t.body).chain(own)) {
            kept.push(c);
        }
    }
    plan.chained = kept;
    Ok(plan.chained.len() < before)
}

/// Whether `p` lies in the tubes `tools` (claude_fusion `_Tubes.holds`): inside one, or on two or more, as an earlier
/// frame's mitred or square corner lies on both its tubes' cut faces and inside neither; on one tool's skin alone (a
/// frame sitting on a tube) it does not. Exact point classification, each tool's box first (a point on its skin lies
/// on the box). A tool the kernel can't bound or classify is passed over, as [`earlier::earlier_tools`] passes over a
/// body it can't classify.
fn holds<'a>(p: Vec3, tools: impl IntoIterator<Item = &'a kernel::Body>) -> bool {
    let mut on = 0;
    for tool in tools {
        let Ok(b) = tool.exact_bounds() else { continue };
        // `!(… <= …)`: a point that isn't finite is in no box.
        if !(p.dist(p.max(b.min).min(b.max)) <= 1e-6) {
            continue;
        }
        match tool.classify_point(p) {
            Ok(kernel::PointClass::Inside) => return true,
            Ok(kernel::PointClass::OnBoundary) => on += 1,
            _ => {}
        }
        if on >= 2 {
            return true;
        }
    }
    false
}

/// The ends a frame doesn't joint within itself, which Fit Ends to Earlier Frames fits to earlier tubes (#4003
/// decision 1), each with whether it is a free end, fitted to this frame's members laid before it too
/// (claude_fusion `fit`: `pool, more = tubes, mine`). Under Auto (`plan`): every end but those of a jointed corner or
/// a chained one; the ends fitted to this frame's own tubes (`plan.fits`: T-junctions, the third and later ends at a
/// point) are not free, as Auto fits them to their own; lone ends and a Free corner's are. Under Mitre, Butt and
/// Coped: the ends at no corner, free. Under None: none. (member, at_start, free).
fn unjointed_ends(f: &FeatureKind, segments: &[(String, Vec3, Vec3)], plan: Option<&Plan>) -> Vec<(usize, bool, bool)> {
    let FeatureKind::FrameMember { corner, .. } = f else { return Vec::new() };
    let (jointed, fitted): (HashSet<(usize, bool)>, HashSet<(usize, bool)>) = match plan {
        Some(plan) => (
            plan.corners
                .iter()
                .filter(|c| c.joint != Joint::Free || c.kind != CornerType::Auto)
                .flat_map(|c| [c.long, c.short])
                .chain(plan.chained.iter().flat_map(|c| [c.a, c.b]))
                .collect(),
            plan.fits.iter().map(|f| (f.member, f.at_start)).collect(),
        ),
        None if *corner == CornerType::None => return Vec::new(),
        None => (frame_corners::find_corners(segments).iter().flat_map(|c| c.ends).collect(), HashSet::new()),
    };
    (0..segments.len())
        .flat_map(|m| [(m, true), (m, false)])
        .filter(|e| !jointed.contains(e))
        .map(|(m, at_start)| (m, at_start, !fitted.contains(&(m, at_start))))
        .collect()
}

/// This frame's own members as tools for its free ends ([`fit_to_earlier`]): each member's place in the laying order,
/// whether member k may reach a point (its line within reach of it, its section and corner extensions counted),
/// member k's tool ([`member_tool`]), and each tool once built.
struct Own<'a> {
    laid: Vec<usize>,
    near: &'a dyn Fn(usize, Vec3) -> bool,
    tool: &'a dyn Fn(usize) -> Result<Traced>,
    tools: Vec<Option<kernel::Body>>,
}

/// `made` (each member as built, by member) with each of `ends` fitted to the earlier Frame Members' tubes
/// ([`frame_fit::fit_end`]), a free end to this frame's members laid before its own too (`own`), each tool built
/// once. A fitted face is named for the end it finishes and the body or line it fits, `corner_start:<body>:face:<k>`
/// (`corner_end:…`). `middle(m)`: a point in member m's material half way along.
fn fit_to_earlier(
    made: &mut [Traced],
    segments: &[(String, Vec3, Vec3)],
    ends: &[(usize, bool, bool)],
    earlier: &[(String, Traced)],
    own: &mut Own,
    middle: &dyn Fn(usize) -> Vec3,
) -> Result<()> {
    let named = |role: &str, name: &str, body: &kernel::Body| {
        let roles = (0..body.face_count()).map(|k| vec![format!("{role}:{name}:face:{k}")]).collect();
        Traced::made(body.clone(), Some(roles))
    };
    for &(m, at_start, free) in ends {
        let (id, a, b) = segments.get(m).ok_or_else(|| DocError::Invalid(format!("no member {m}")))?;
        let (end, toward, role) = if at_start { (*a, *a - *b, "corner_start") } else { (*b, *b - *a, "corner_end") };
        let mut against: Vec<Traced> = earlier.iter().map(|(body, t)| named(role, body, &t.body)).collect();
        let place = |k: usize| own.laid.get(k).copied().unwrap_or(usize::MAX);
        for k in (0..segments.len()).filter(|&k| free && k != m && place(k) < place(m) && (own.near)(k, end)) {
            let (Some(slot), Some((line, _, _))) = (own.tools.get_mut(k), segments.get(k)) else { continue };
            if slot.is_none() {
                *slot = Some((own.tool)(k)?.body);
            }
            if let Some(body) = slot.as_ref() {
                against.push(named(role, line, body));
            }
        }
        if against.is_empty() {
            continue;
        }
        let against: Vec<&Traced> = against.iter().collect();
        let member = made.get_mut(m).ok_or_else(|| DocError::Invalid(format!("no member {m}")))?;
        *member = frame_fit::fit_end(member, end, toward, middle(m), &against, id)?;
    }
    Ok(())
}

/// A member feature's lines (in its component frame), profile regions, default name, the
/// profile point that sits on the lines, the profile's turn about them and the frame sketch's x
/// axis (`None`: a 3D sketch, or no sketch), each member's place in the drawing order, and each
/// member's sketch plane ([`segments`]).
struct Layout {
    segments: Vec<(String, Vec3, Vec3)>,
    regions: Vec<Region2>,
    description: String,
    at: Vec2,
    turn: f64,
    sketch_x: Option<Vec3>,
    drawn: Vec<usize>,
    planes: Vec<MemberPlane>,
}

fn layout(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Layout> {
    let FeatureKind::FrameMember { profile, anchor, locate, angle, mirror, corner, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let (segments, sketch_x, drawn, planes) = segments(doc, vals, f, st)?;
    let (mut regions, description, origin) = profile_regions(vals, st, profile)?;
    if *mirror {
        regions = regions.iter().map(frame::mirrored).collect();
    }
    let at = match locate {
        Some([x, y]) => {
            // A design file is hostile input: the command checks the point, a file may not.
            if !(x.is_finite() && y.is_finite() && x.abs() <= frame::MAX_PROFILE_SIZE && y.abs() <= frame::MAX_PROFILE_SIZE) {
                return Err(DocError::Invalid(format!("frame member `locate` must be two finite lengths within {} mm", frame::MAX_PROFILE_SIZE)));
            }
            // `locate` is the profile point that sits on the line, measured from the profile's
            // own origin (`Profile::origin`). The built-in region is centred on its bounding
            // box, so that origin moves when the section's size changes, and the located point
            // moves with it: the member stays attached at the same point of its section.
            let p = origin + Vec2::new(*x, *y);
            if *mirror { Vec2::new(-p.x, p.y) } else { p }
        }
        None => frame::anchor_point(&regions, *anchor),
    };
    let turn = match angle {
        Some(e) => val(vals, e, Kind::Angle)?,
        None => 0.0,
    };
    if *corner == CornerType::Auto {
        // A full circle is one arc, and its tube's one wall has its seam where the arc starts. Unmirrored and
        // unturned that is the section's +x, the inside of an outline Auto lays anticlockwise; Mirror or a turn
        // carries it round, to the outside on a mirror or a half turn, where each through tube's flat end touches
        // the tube fitted to it along that tube's outer side. SMLib's check refuses the fit there ("Face Near
        // missing Seam"), so under Auto every circle keeps its seam on the section's +x as placed (#4003).
        regions = regions.iter().map(|r| seamed(r, -turn)).collect();
    }
    Ok(Layout { segments, regions, description, at, turn, sketch_x, drawn, planes })
}

/// `region` with each full circle (a loop of one whole-turn arc) starting at angle `start`, turning the way it did.
fn seamed(region: &Region2, start: f64) -> Region2 {
    let seam = |l: &Loop2| match l.segs.as_slice() {
        [Seg2::Arc { center, radius, sweep, .. }] if (sweep.abs() - TAU).abs() < 1e-9 => {
            Loop2 { segs: vec![Seg2::Arc { center: *center, radius: *radius, start, sweep: *sweep }] }
        }
        _ => l.clone(),
    };
    Region2 { outer: seam(&region.outer), holes: region.holes.iter().map(seam).collect() }
}

/// Each member's line in the feature's component frame: (member id, start, end), the sketch's
/// lines then the pairs'; the frame sketch's x axis (`None`: a 3D sketch, or no sketch); each
/// member's place in the sketch's drawing order ([`drawn_order`]); and each member's sketch plane:
/// a 2D sketch's own front normal (x × y, whichever way a face it is drawn on faces) and x axis
/// for its lines, `None` for a 3D sketch's lines and for pairs.
fn segments(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Path> {
    let FeatureKind::FrameMember { path_sketch, lines, pairs, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let (mut segments, sketch, plane) = match path_sketch {
        Some(path_sketch) => {
            let ps =
                st.sketch(path_sketch.0).ok_or_else(|| DocError::Unknown(format!("sketch {path_sketch} (it must come earlier in the timeline)")))?;
            // The frame sketch in this feature's component frame: its plane, and its 3D lines.
            let mapped = if ps.sketch.wires.is_empty() {
                Cow::Owned(sketch_in_feature_frame(doc, ps, f.component)?)
            } else {
                path_in_frame(doc, ps, f.component)?
            };
            let plane = ps.sketch.wires.is_empty().then(|| (mapped.plane.normal(), mapped.plane.x));
            (member_lines(mapped.as_ref(), lines)?, Some(&ps.sketch), plane)
        }
        None => (Vec::new(), None, None),
    };
    let mut planes = vec![plane; segments.len()];
    segments.extend(pair_lines(doc, vals, f, st, pairs)?);
    planes.resize(segments.len(), None);
    let mut seen = HashSet::new();
    if let Some((id, _, _)) = segments.iter().find(|(id, _, _)| !seen.insert(id.as_str())) {
        return Err(DocError::Invalid(format!("member id `{id}` is used twice")));
    }
    if segments.is_empty() {
        return Err(DocError::Invalid("the frame member has no members (give it a sketch or pairs of points)".into()));
    }
    if segments.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature")));
    }
    let drawn = sketch.map(|sk| drawn_order(sk, &segments)).unwrap_or_default();
    Ok((segments, plane.map(|p| p.1), drawn, planes))
}

/// A Frame Member's members, as [`segments`] finds them.
type Path = (Vec<(String, Vec3, Vec3)>, Option<Vec3>, Vec<usize>, Vec<MemberPlane>);

/// Each member's centreline where the view shows it: (member id, start, end), in the world.
/// `st` is the model the feature was evaluated on (`Model::state_before`).
pub fn member_segments(doc: &Document, st: &ModelState, f: &Feature) -> Result<Vec<(String, Vec3, Vec3)>> {
    let m = doc.component_transform(f.component);
    let place = |p: Vec3| crate::apply_point(&m, p);
    Ok(segments(doc, &doc.evaluation_values(), f, st)?.0.into_iter().map(|(id, a, b)| (id, place(a), place(b))).collect())
}

/// The member of feature `f` on line `line`, as its body was made: the plane square to the line
/// at its start as cut (x × y along the line; a square corner cut moves the start to the cut), its
/// length as cut, its profile in that plane, and whether an End Cap may go on its [start, end]:
/// an uncut end, or a through member's end of a Butt or Coped corner, Auto's included (not the
/// butting member's end, which lies against the through member, a cope, a fit or a mitre).
pub(crate) fn member_section(
    doc: &Document,
    vals: &BTreeMap<String, Value>,
    f: &Feature,
    st: &ModelState,
    line: &str,
) -> Result<(Plane, f64, Region2, [bool; 2])> {
    let FeatureKind::FrameMember { corner, corners, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let Layout { segments, regions, at, turn, sketch_x, drawn, planes, .. } = layout(doc, vals, f, st)?;
    let m = segments.iter().position(|(id, _, _)| id == line).ok_or_else(|| DocError::Unknown(format!("line `{line}` of the member feature")))?;
    let plan = auto_plan(*corner, &segments, &planes, corners, &drawn)?;
    let (axes, _) = section_axes(&segments, &planes, plan.as_ref(), turn)?;
    let (x, y) = axes.get(m).copied().ok_or_else(|| DocError::Invalid(format!("line `{line}` has no length")))?;
    let (a, b) = segments.get(m).map(|s| (s.1, s.2)).ok_or_else(|| DocError::Unknown(format!("line `{line}` of the member feature")))?;
    let placed = placer(&axes, at);
    // Each end where the member is cut, when the cut is square to its line (a mitre is not: End
    // Cap refuses it from the face itself).
    // ponytail: a corner or chain corner Fit Ends turns Free (inside a tube) is read here as jointed, as is an end
    // fitted to a tube as open; finding them builds every earlier tool, which End Cap would pay on every pick.
    let ends = frame_ends(&f.kind, &segments, &regions, &placed, plan, (sketch_x, &drawn))?;
    let dir = (b - a).normalized().ok_or_else(|| DocError::Invalid(format!("line `{line}` has no length")))?;
    let [start, end] = ends.get(m).copied().unwrap_or([None, None]);
    let square = |e: Option<CutEnd>, p: Vec3| match e {
        Some((Some(c), _, _)) if c.normal.dot(dir).abs() > 1.0 - 1e-9 => (c.origin - p).dot(dir),
        _ => 0.0,
    };
    let (lo, hi) = (square(start, a), square(end, b));
    let plane =
        Plane::new(a + dir * lo - x * at.x - y * at.y, x, y).ok_or_else(|| DocError::Invalid(format!("line `{line}`: no plane square to it")))?;
    let (plane, sections) = along(plane, &regions, dir);
    let region = sections.first().cloned().ok_or_else(|| DocError::Invalid("the member profile has no area".into()))?;
    let open = |e: Option<CutEnd>| e.is_none_or(|e| e.2);
    Ok((plane, a.dist(b) + hi - lo, region, [open(start), open(end)]))
}

/// The body name of the member on each of `lines`: its own name from `pinned` (by line id) when no
/// other body has it, else the first free one of `base`, `base (2)`, `base (3)`… that no line has
/// pinned. A member therefore keeps its name when other lines are added, deleted or reordered.
pub fn member_names(st: &ModelState, lines: &[String], pinned: &BTreeMap<String, String>, base: &str) -> Vec<String> {
    let reserved: HashSet<&str> = pinned.values().map(String::as_str).collect();
    let mut used: HashSet<String> = HashSet::new();
    let mut out: Vec<Option<String>> = lines
        .iter()
        .map(|line| {
            let n = pinned.get(line).map(|n| n.trim()).filter(|n| !n.is_empty() && st.body(n).is_none())?;
            used.insert(n.to_string()).then(|| n.to_string())
        })
        .collect();
    let base = if base.trim().is_empty() { "Member" } else { base.trim() };
    // Each candidate is refused at most once per body, used or reserved name: past that many,
    // a free one has been found for every member.
    let limit = st.bodies.len() + reserved.len() + lines.len() + 2;
    let mut next = 1usize;
    for slot in out.iter_mut().filter(|n| n.is_none()) {
        while next <= limit {
            let candidate = if next == 1 { base.to_string() } else { format!("{base} ({next})") };
            next += 1;
            if st.body(&candidate).is_none() && !used.contains(&candidate) && !reserved.contains(candidate.as_str()) {
                used.insert(candidate.clone());
                *slot = Some(candidate);
                break;
            }
        }
    }
    out.into_iter().map(|n| n.unwrap_or_else(|| base.to_string())).collect()
}

/// The ids of a `FrameMember` feature's lines in `st`, then its pairs', in the order its members
/// are made.
pub fn member_line_ids(st: &ModelState, kind: &FeatureKind) -> Result<Vec<String>> {
    let FeatureKind::FrameMember { path_sketch, lines, pairs, .. } = kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let mut ids: Vec<String> = match path_sketch {
        Some(path_sketch) => {
            let ps = st.sketch(path_sketch.0).ok_or_else(|| DocError::Unknown(format!("sketch {path_sketch}")))?;
            member_lines(ps, lines)?.into_iter().map(|(id, _, _)| id).collect()
        }
        None => Vec::new(),
    };
    ids.extend(pairs.iter().map(|p| p.id.clone()));
    Ok(ids)
}

/// The member (line or pair id) of Frame Member `f` whose body is `body`, if one is: by position
/// when the feature names its bodies itself, else by its `names` entry.
pub fn member_of_body(st: &ModelState, f: &Feature, body: &str) -> Result<Option<String>> {
    let FeatureKind::FrameMember { names, .. } = &f.kind else { return Ok(None) };
    let ids = member_line_ids(st, &f.kind)?;
    Ok(if f.body_names.is_empty() {
        ids.into_iter().find(|line| names.get(line).is_some_and(|name| name == body))
    } else {
        f.body_names.iter().position(|name| name == body).and_then(|i| ids.into_iter().nth(i))
    })
}

/// Each pair's ends, found again as construction geometry finds its points: in this feature's
/// component frame, where its lines are laid out, so both take the same "up". A pair whose point
/// is gone, or whose ends now meet, fails the feature, naming the pair.
fn pair_lines(
    doc: &Document,
    vals: &BTreeMap<String, Value>,
    f: &Feature,
    st: &ModelState,
    pairs: &[MemberPair],
) -> Result<Vec<(String, Vec3, Vec3)>> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    if pairs.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature")));
    }
    let cst = super::construction_state(doc, st, f.component)?;
    let ctx = crate::construct::Ctx { doc, vals, st: &cst, depth: 0, component: f.component };
    pairs
        .iter()
        .map(|p| {
            let end = |g, which: &str| match crate::construct::point(&ctx, g) {
                Ok(v) if v.is_finite() => Ok(v),
                Ok(_) => Err(DocError::Invalid(format!("{}: its {which} point is not a finite point", p.id))),
                Err(e) => Err(DocError::Invalid(format!("{}: its {which} point is gone ({e})", p.id))),
            };
            let (a, b) = (end(&p.a, "first")?, end(&p.b, "second")?);
            if !(a.dist(b) > MIN_LENGTH) {
                return Err(DocError::Invalid(format!("{}: its two points are in one place (it has no length)", p.id)));
            }
            Ok((p.id.clone(), a, b))
        })
        .collect()
}

/// The member lines that end at the sketch's point `point` (a point id, or a 3D sketch's
/// `wire:index` node), among `lines` (none: every line of the sketch, as a feature with no lines
/// takes them). A Corner Override names its corner by these. Under `auto`, two or more lines end
/// there and none passes through (spec decision 3); otherwise exactly two, a corner as
/// [`frame_corners::find_corners`] finds them. Only the sketch's lines are looked at: a corner where
/// a point-to-point member ends is named through `corners` with the pair's id.
pub fn corner_at_point(st: &ModelState, sketch: u64, lines: &[String], point: &str, auto: bool) -> Result<Vec<String>> {
    let ss = st.sketch(sketch).ok_or_else(|| DocError::Unknown(format!("sketch {sketch}")))?;
    let at =
        crate::construct::sketch_point_position(st, sketch, point).ok_or_else(|| DocError::Unknown(format!("point `{point}` in {}", ss.name)))?;
    let segments = member_lines(ss, lines)?;
    let tol = frame_corners::CORNER_TOL;
    if auto {
        let ends = |(_, a, b): &&(String, Vec3, Vec3)| a.dist(at) <= tol || b.dist(at) <= tol;
        let ending: Vec<String> = segments.iter().filter(ends).map(|s| s.0.clone()).collect();
        let through = segments.iter().any(|s| !ends(&s) && at.dist_to_segment(s.1, s.2) <= tol);
        if ending.len() < 2 || through {
            return Err(DocError::Invalid("Pick a corner of the frame: a point where two or more members end".into()));
        }
        return Ok(ending);
    }
    let not_a_corner = || DocError::Invalid("Pick a corner of the frame: a point where exactly two members end".into());
    let corners = frame_corners::find_corners(&segments);
    let corner = corners.iter().find(|c| c.point.dist(at) <= tol).ok_or_else(not_a_corner)?;
    let [(a, _), (b, _)] = corner.ends;
    let id = |m: usize| segments.get(m).map(|s| s.0.clone());
    id(a).zip(id(b)).map(|(a, b)| vec![a, b]).ok_or_else(not_a_corner)
}

/// The sketch point where the lines `members` (two or more) all end together (an end point of
/// one of them, named as a pick names it), or `None` when they no longer meet.
pub fn corner_point(st: &ModelState, sketch: u64, members: &[String]) -> Option<String> {
    let ss = st.sketch(sketch)?;
    if members.len() < 2 {
        return None;
    }
    let lines = member_lines(ss, members).ok()?;
    let tol = frame_corners::CORNER_TOL;
    let (_, a, b) = lines.first()?;
    let point = [*a, *b].into_iter().find(|&p| lines.iter().all(|(_, a, b)| a.dist(p) <= tol || b.dist(p) <= tol))?;
    let sk = &ss.sketch;
    let ends = |id: &String| -> Vec<String> {
        if let Some(w) = sk.wire_index(id).and_then(|i| sk.wires.get(i)) {
            return vec![format!("{id}:0"), format!("{id}:{}", w.fit.len().saturating_sub(1))];
        }
        let ends = sk.curve_index(id).and_then(|i| sk.curves.get(i)).and_then(|c| c.kind.ends());
        ends.map(|(a, b)| [a, b].iter().filter_map(|&i| sk.points.get(i).map(|p| p.id.clone())).collect()).unwrap_or_default()
    };
    members.iter().flat_map(ends).find(|p| crate::construct::sketch_point_position(st, sketch, p).is_some_and(|at| at.dist(point) <= tol))
}

/// Whether curve `ci` of `sk` is one a frame member is laid on unless lines are picked: a straight
/// line drawn in it (not construction, not a centreline, not projected or otherwise linked). A
/// marquee in the Frame Member dialog picks these.
pub fn is_member_line(sk: &Sketch, ci: usize) -> bool {
    sk.curves.get(ci).is_some_and(|c| !c.construction && !c.centerline && c.link.is_none() && matches!(c.kind, CurveKind::Line { .. }))
}

/// Each member's place in the sketch's drawing order: a 2D line's curve index, a 3D line's wire
/// index after them (looked up as [`member_lines`] looks them up). A pair's member isn't drawn in
/// the sketch: it ranks after every line, and pairs tie among themselves, which `frame_corners::
/// through` then settles by member index, the pairs' own order. (With no sketch, `drawn` is empty
/// and every member ranks by its index alike.)
fn drawn_order(sk: &Sketch, segments: &[(String, Vec3, Vec3)]) -> Vec<usize> {
    let rank = |id: &str| sk.wire_index(id).map(|w| sk.curves.len() + w).or_else(|| sk.curve_index(id)).unwrap_or(usize::MAX);
    segments.iter().map(|(id, _, _)| rank(id)).collect()
}

/// The member lines of the frame sketch, each as (curve id, start, end) in its frame: the lines
/// asked for, or every straight line drawn in it ([`is_member_line`], with a length).
fn member_lines(ss: &SolvedSketch, ids: &[String]) -> Result<Vec<(String, Vec3, Vec3)>> {
    let sk = &ss.sketch;
    let mut out = Vec::new();
    if ids.is_empty() {
        for (ci, c) in sk.curves.iter().enumerate() {
            if is_member_line(sk, ci)
                && let Some((a, b)) = planar_line(ss, ci)
                && a.dist(b) > MIN_LENGTH
            {
                out.push((c.id.clone(), a, b));
            }
        }
        for w in sk.wires.iter().filter(|w| w.link.is_none()) {
            if let Some((a, b)) = straight(&w.pts)
                && a.dist(b) > MIN_LENGTH
            {
                out.push((w.id.clone(), a, b));
            }
        }
        if out.is_empty() {
            return Err(DocError::Invalid(format!("{} has no lines for members", ss.name)));
        }
        if out.len() > MAX_MEMBERS {
            return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature (pick the lines)")));
        }
        return Ok(out);
    }
    if ids.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature")));
    }
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            return Err(DocError::Invalid(format!("line `{id}` is listed twice")));
        }
        let line = if let Some(wi) = sk.wire_index(id) {
            sk.wires.get(wi).and_then(|w| straight(&w.pts))
        } else {
            let ci = sk.curve_index(id).ok_or_else(|| DocError::Unknown(format!("line `{id}` in {}", ss.name)))?;
            planar_line(ss, ci)
        };
        let (a, b) = line.ok_or_else(|| DocError::Invalid(format!("`{id}` is not a straight line (members run along lines)")))?;
        if !(a.dist(b) > MIN_LENGTH) {
            return Err(DocError::Invalid(format!("line `{id}` has no length")));
        }
        out.push((id.clone(), a, b));
    }
    Ok(out)
}

/// A sketch line's ends in the sketch's frame.
fn planar_line(ss: &SolvedSketch, ci: usize) -> Option<(Vec3, Vec3)> {
    let CurveKind::Line { a, b } = ss.sketch.curves.get(ci)?.kind else { return None };
    let (pa, pb) = (ss.sketch.points.get(a)?.pos, ss.sketch.points.get(b)?.pos);
    Some((ss.plane.to_world(pa), ss.plane.to_world(pb)))
}

/// The ends of a 3D polyline that runs straight from its first point to its last: every point
/// within [`STRAIGHT_TOL`] of the line between them, none behind the start, past the end or back
/// from a point before it.
fn straight(pts: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let (a, b) = (*pts.first()?, *pts.last()?);
    let (len, d) = (a.dist(b), (b - a).normalized()?);
    let mut reached = 0.0_f64;
    for p in pts {
        let q = *p - a;
        let t = q.dot(d);
        if (q - d * t).len() > STRAIGHT_TOL || t < reached - STRAIGHT_TOL || t > len + STRAIGHT_TOL {
            return None;
        }
        reached = reached.max(t);
    }
    Some((a, b))
}

/// The regions of a member's profile, in the profile's own coordinates: one piece of material,
/// holes and all; and what its members are named after when they have no name of their own.
fn profile_regions(vals: &BTreeMap<String, Value>, st: &ModelState, profile: &MemberProfile) -> Result<(Vec<Region2>, String, Vec2)> {
    let (regions, description, origin) = match profile {
        MemberProfile::Library { profile } => {
            let p = profile.resolve(|name, e, kind| val(vals, e, kind).map_err(|err| DocError::Invalid(format!("profile `{name}`: {err}"))))?;
            let origin = p.origin();
            (vec![p.region()?], profile.size.clone().unwrap_or_else(|| p.description()), origin)
        }
        MemberProfile::Sketch { sketch, profiles } => {
            let ss = st.sketch(sketch.0).ok_or_else(|| DocError::Unknown(format!("sketch {sketch} (it must come earlier in the timeline)")))?;
            let picked = match profiles {
                ProfileSel::All => material(ss)?,
                sel => select_profiles(ss, sel)?,
            };
            (solidcraft_sketch::merge_regions(&picked), ss.name.clone(), Vec2::ZERO)
        }
    };
    match regions.len() {
        1 => Ok((regions, description, origin)),
        0 => Err(DocError::Invalid("the member profile has no area".into())),
        n => Err(DocError::Invalid(format!("the member profile is {n} separate pieces (pick one with `profiles`)"))),
    }
}

/// The material of a profile sketch: of what was drawn (as `solid.extrude` takes all profiles:
/// projected reference geometry only when the drawn curves make no profile, letters without
/// their counters filled in), each profile inside an even number of other profiles' outlines,
/// so nested loops alternate between material and hole.
fn material(ss: &SolvedSketch) -> Result<Vec<Region2>> {
    let drawn = solidcraft_sketch::find_drawn_profiles(&ss.sketch);
    let ps = solidcraft_sketch::without_text_counters(&ss.sketch, if drawn.is_empty() { ss.profiles.clone() } else { drawn });
    if ps.is_empty() {
        return Err(DocError::Invalid(format!("{} has no closed profiles", ss.name)));
    }
    if ps.len() > MAX_PROFILE_PIECES {
        return Err(DocError::Invalid(format!("{} has more than {MAX_PROFILE_PIECES} profiles (pick the member's with `profiles`)", ss.name)));
    }
    // Outer polylines and their boxes, once.
    let polys: Vec<Ring<Vec2>> = ps.iter().map(|p| p.region.outer.polyline(1e-3)).collect();
    let boxes: Vec<Aabb2> = polys.iter().map(|poly| poly.bounds()).collect();
    Ok(ps
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            let at = p.region.interior_point();
            let depth = polys.iter().zip(&boxes).enumerate().filter(|(j, (poly, bb))| j != i && bb.contains(at) && poly.contains(at)).count();
            depth % 2 == 0
        })
        .map(|(_, p)| p.region.clone())
        .collect())
}
