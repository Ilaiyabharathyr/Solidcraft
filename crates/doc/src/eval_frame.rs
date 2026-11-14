//! Frame members (#4003): one body per straight line of a frame sketch, and per pair of points
//! (#15705), its cross-section a profile laid square to the line (`frame::member_axes`) and run
//! from one end to the other.

use solidcraft_geom::prelude::*;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use solidcore_record::{Plane, Region2, Vec2, Vec3};
use solidcraft_geom::{Aabb2, Ring};
use solidcraft_kernel::{self as kernel, BoolOp};
use solidcraft_sketch::{CurveKind, Sketch};

use super::{ModelBody, ModelState, SolvedSketch, new_name, path_in_frame, select_profiles, sketch_in_feature_frame, val};
use crate::expr::{Kind, Value};
use crate::frame::{self, CornerOverride, CornerType, MAX_MEMBERS, MemberPair, MemberProfile};
use crate::frame_corners::{self, Cut};
use crate::lineage::Traced;
use crate::{DocError, Document, Feature, FeatureKind, ProfileSel, Result};

/// Most profiles a profile sketch may have for its material to be worked out.
const MAX_PROFILE_PIECES: usize = 256;
/// Shortest member (mm).
const MIN_LENGTH: f64 = 1e-6;
/// How far (mm) a 3D sketch curve may stray from the line between its ends and still be a line.
const STRAIGHT_TOL: f64 = 1e-3;

/// Evaluate a `FrameMember` feature: one new body per line, each named
/// ([`member_names`], or by position for a copy whose `body_names` were set).
pub(super) fn evaluate(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &mut ModelState) -> Result<()> {
    let FeatureKind::FrameMember { body_name, names, corner, flip, corners, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let Layout { segments, regions, description, at, turn, sketch_x, drawn } = layout(doc, vals, f, st)?;
    let mut axes = Vec::with_capacity(segments.len());
    for (id, a, b) in &segments {
        axes.push(frame::member_axes(*b - *a, turn).ok_or_else(|| DocError::Invalid(format!("line `{id}` has no length")))?);
    }
    // The member's section plane, square to its line, placed at `p` on the line.
    let placed = |m: usize, p: Vec3| {
        let (x, y) = axes.get(m).copied().unwrap_or((Vec3::X, Vec3::Y));
        let origin = p - x * at.x - y * at.y;
        Plane::new(origin, x, y).unwrap_or(Plane { origin, x, y })
    };
    // A frame with no corners to finish is built exactly as it always was.
    // Until the Auto rule is built, Auto builds exactly as None.
    let mut cut = if corner.is_none() && corners.is_empty() {
        Vec::new()
    } else {
        cut_members(&segments, &regions, &placed, (*corner, *flip), corners, (sketch_x, &drawn))?
    };
    let mut made = Vec::with_capacity(segments.len());
    for (m, (id, a, b)) in segments.iter().enumerate() {
        if let Some(t) = cut.get_mut(m).and_then(Option::take) {
            made.push(t);
            continue;
        }
        let (x, y) = axes.get(m).copied().ok_or_else(|| DocError::Invalid(format!("line `{id}` has no length")))?;
        let plane = Plane::new(*a - x * at.x - y * at.y, x, y).ok_or_else(|| DocError::Invalid(format!("line `{id}`: no plane square to it")))?;
        made.push(crate::lineage::numbered_tool(one_solid(kernel::extrude(&plane, &regions, 0.0, a.dist(*b))?, id)?));
    }
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
        // Two members: the corner they meet at. Three or more are Auto's (not built yet).
        let [a, b] = o.members.as_slice() else {
            if o.members.len() < 2 {
                return Err(DocError::Invalid("A corner override names at least two members".into()));
            }
            // Three or more are Auto's alone; Auto builds as None until its rule is built.
            if finish.0 == CornerType::Auto {
                continue;
            }
            return Err(DocError::Invalid("A corner override is not at a corner of the frame".into()));
        };
        let k = at.get(&pair(a, b)).ok_or_else(|| DocError::Invalid("A corner override is not at a corner of the frame".into()))?;
        if let Some(f) = finishes.get_mut(*k) {
            *f = (o.corner, o.flip);
        }
    }
    let mut ends: Vec<[Option<CutEnd>; 2]> = vec![[None, None]; segments.len()];
    // Each Coped Butt's (T, U): U loses T's filled shape once every plane cut is made.
    let mut copes = Vec::new();
    for (c, (kind, flip)) in corners.iter().zip(finishes) {
        let (cuts, cope) = frame_corners::corner_cuts(kind, flip, c, segments, placed, regions, sketch_x, drawn)?;
        for (k, (m, at_start, cut)) in cuts.into_iter().enumerate() {
            let away = frame_corners::away(segments, (m, at_start));
            let ext = away
                .and_then(|w| frame_corners::extension(&placed(m, c.point), regions, c.point, w, &cut))
                .ok_or_else(|| DocError::Invalid(format!("the member on line `{}` can't be built out to its corner cut", id(m))))?;
            // A butt's first cut is T's. T must not run back past the far end of U, the corner's other member.
            if k == 0 && kind.flips() {
                let [(e0, _), (e1, _)] = c.ends;
                let u = if e0 == m { e1 } else { e0 };
                let u_len = segments.get(u).map_or(0.0, |(_, a, b)| a.dist(*b));
                let u_reach = away.map_or(0.0, |w| frame_corners::reach(&placed(u, c.point), regions, c.point, -w));
                if ext - 1.0 > u_len + u_reach {
                    return Err(frame_corners::corner_error(c.point, "too flat to butt"));
                }
            }
            // A butt's first cut is T's, square to its line: the one cut end a cap may go on.
            let cap = k == 0 && kind.flips();
            if let Some(end) = ends.get_mut(m).and_then(|e| e.get_mut(usize::from(!at_start))) {
                *end = Some((Some(cut), ext, cap));
            }
        }
        let Some((t, u)) = cope else { continue };
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
    }
    Ok((ends, copes))
}

/// Each member cut at its corners (#4003), or `None` for a member no corner cuts (built square,
/// as without corners). A corner is finished as its override in `overrides` says (the override
/// naming its two lines, in either order), else as `finish` (type, flip) says. A cut member is
/// built past each cut end far enough to reach the whole cut, then cut there; the cut face is
/// named `corner_start` or `corner_end`. A Coped Butt's U is built out across T and, after every
/// plane cut, loses T's filled shape, keeping its largest piece. `placed(m, p)` is member m's
/// section plane, square to its line, placed at `p`.
fn cut_members(
    segments: &[(String, Vec3, Vec3)],
    regions: &[Region2],
    placed: &dyn Fn(usize, Vec3) -> Plane,
    finish: (CornerType, bool),
    overrides: &[CornerOverride],
    tie: Tie,
) -> Result<Vec<Option<Traced>>> {
    let (ends, copes) = corner_ends(segments, regions, placed, finish, overrides, tie)?;
    let id = |m: usize| segments.get(m).map_or("", |s| s.0.as_str());
    // Member m built out at its cut ends and cut there, from `regions`.
    let build = |m: usize, regions: &[Region2]| -> Result<Traced> {
        let ((id, a, b), [start, end]) = segments.get(m).zip(ends.get(m)).ok_or_else(|| DocError::Invalid(format!("no member {m}")))?;
        let (lo, hi) = (start.map_or(0.0, |c| c.1), end.map_or(0.0, |c| c.1));
        let mut t = crate::lineage::numbered_tool(one_solid(kernel::extrude(&placed(m, *a), regions, -lo, a.dist(*b) + hi)?, id)?);
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

/// A member feature's lines (in its component frame), profile regions, default name, the
/// profile point that sits on the lines, the profile's turn about them and the frame sketch's x
/// axis (`None`: a 3D sketch, or no sketch), and each member's place in the drawing order.
struct Layout {
    segments: Vec<(String, Vec3, Vec3)>,
    regions: Vec<Region2>,
    description: String,
    at: Vec2,
    turn: f64,
    sketch_x: Option<Vec3>,
    drawn: Vec<usize>,
}

fn layout(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Layout> {
    let FeatureKind::FrameMember { profile, anchor, locate, angle, mirror, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let (segments, sketch_x, drawn) = segments(doc, vals, f, st)?;
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
    Ok(Layout { segments, regions, description, at, turn, sketch_x, drawn })
}

/// Each member's line in the feature's component frame: (member id, start, end), the sketch's
/// lines then the pairs'; the frame sketch's x axis (`None`: a 3D sketch, or no sketch); and each
/// member's place in the sketch's drawing order ([`drawn_order`]).
fn segments(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Path> {
    let FeatureKind::FrameMember { path_sketch, lines, pairs, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let (mut segments, sketch, sketch_x) = match path_sketch {
        Some(path_sketch) => {
            let ps =
                st.sketch(path_sketch.0).ok_or_else(|| DocError::Unknown(format!("sketch {path_sketch} (it must come earlier in the timeline)")))?;
            // The frame sketch in this feature's component frame: its plane, and its 3D lines.
            let mapped = if ps.sketch.wires.is_empty() {
                Cow::Owned(sketch_in_feature_frame(doc, ps, f.component)?)
            } else {
                path_in_frame(doc, ps, f.component)?
            };
            (member_lines(mapped.as_ref(), lines)?, Some(&ps.sketch), ps.sketch.wires.is_empty().then_some(mapped.plane.x))
        }
        None => (Vec::new(), None, None),
    };
    segments.extend(pair_lines(doc, vals, f, st, pairs)?);
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
    Ok((segments, sketch_x, drawn))
}

/// A Frame Member's members, as [`segments`] finds them.
type Path = (Vec<(String, Vec3, Vec3)>, Option<Vec3>, Vec<usize>);

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
/// an uncut end, or a through member's end of a Butt or Coped corner (not the butting member's
/// end, which lies against the through member, a cope or a mitre).
pub(crate) fn member_section(
    doc: &Document,
    vals: &BTreeMap<String, Value>,
    f: &Feature,
    st: &ModelState,
    line: &str,
) -> Result<(Plane, f64, Region2, [bool; 2])> {
    let FeatureKind::FrameMember { corner, flip, corners, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let Layout { segments, regions, at, turn, sketch_x, drawn, .. } = layout(doc, vals, f, st)?;
    let m = segments.iter().position(|(id, _, _)| id == line).ok_or_else(|| DocError::Unknown(format!("line `{line}` of the member feature")))?;
    let mut axes = Vec::with_capacity(segments.len());
    for (id, a, b) in &segments {
        axes.push(frame::member_axes(*b - *a, turn).ok_or_else(|| DocError::Invalid(format!("line `{id}` has no length")))?);
    }
    let (x, y) = axes.get(m).copied().ok_or_else(|| DocError::Invalid(format!("line `{line}` has no length")))?;
    let (a, b) = segments.get(m).map(|s| (s.1, s.2)).ok_or_else(|| DocError::Unknown(format!("line `{line}` of the member feature")))?;
    let placed = |k: usize, p: Vec3| {
        let (x, y) = axes.get(k).copied().unwrap_or((Vec3::X, Vec3::Y));
        let origin = p - x * at.x - y * at.y;
        Plane::new(origin, x, y).unwrap_or(Plane { origin, x, y })
    };
    // Each end where the member is cut, when the cut is square to its line (a mitre is not: End
    // Cap refuses it from the face itself).
    let ends = if corner.is_none() && corners.is_empty() {
        Vec::new()
    } else {
        corner_ends(&segments, &regions, &placed, (*corner, *flip), corners, (sketch_x, &drawn))?.0
    };
    let dir = (b - a).normalized().ok_or_else(|| DocError::Invalid(format!("line `{line}` has no length")))?;
    let [start, end] = ends.get(m).copied().unwrap_or([None, None]);
    let square = |e: Option<CutEnd>, p: Vec3| match e {
        Some((Some(c), _, _)) if c.normal.dot(dir).abs() > 1.0 - 1e-9 => (c.origin - p).dot(dir),
        _ => 0.0,
    };
    let (lo, hi) = (square(start, a), square(end, b));
    let plane =
        Plane::new(a + dir * lo - x * at.x - y * at.y, x, y).ok_or_else(|| DocError::Invalid(format!("line `{line}`: no plane square to it")))?;
    let region = regions.into_iter().next().ok_or_else(|| DocError::Invalid("the member profile has no area".into()))?;
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

/// The two member lines that end at the sketch's point `point` (a point id, or a 3D sketch's
/// `wire:index` node): a corner as [`frame_corners::find_corners`] finds them among `lines` (none:
/// every line of the sketch, as a feature with no lines takes them). A Corner Override names its
/// corner by these. Only the sketch's lines are looked at: a corner where a point-to-point member
/// ends is named through `corners` with the pair's id.
pub fn corner_at_point(st: &ModelState, sketch: u64, lines: &[String], point: &str) -> Result<[String; 2]> {
    let ss = st.sketch(sketch).ok_or_else(|| DocError::Unknown(format!("sketch {sketch}")))?;
    let at =
        crate::construct::sketch_point_position(st, sketch, point).ok_or_else(|| DocError::Unknown(format!("point `{point}` in {}", ss.name)))?;
    let segments = member_lines(ss, lines)?;
    let not_a_corner = || DocError::Invalid("Pick a corner of the frame: a point where exactly two members end".into());
    let corners = frame_corners::find_corners(&segments);
    let corner = corners.iter().find(|c| c.point.dist(at) <= frame_corners::CORNER_TOL).ok_or_else(not_a_corner)?;
    let [(a, _), (b, _)] = corner.ends;
    let id = |m: usize| segments.get(m).map(|s| s.0.clone());
    id(a).zip(id(b)).map(|(a, b)| [a, b]).ok_or_else(not_a_corner)
}

/// The sketch point at the corner where the lines `pair` end together (an end point of either
/// line, named as a pick names it), or `None` when they no longer meet.
pub fn corner_point(st: &ModelState, sketch: u64, pair: &[String]) -> Option<String> {
    let ss = st.sketch(sketch)?;
    let corner = frame_corners::find_corners(&member_lines(ss, pair).ok()?).into_iter().next()?;
    let sk = &ss.sketch;
    let ends = |id: &String| -> Vec<String> {
        if let Some(w) = sk.wire_index(id).and_then(|i| sk.wires.get(i)) {
            return vec![format!("{id}:0"), format!("{id}:{}", w.fit.len().saturating_sub(1))];
        }
        let ends = sk.curve_index(id).and_then(|i| sk.curves.get(i)).and_then(|c| c.kind.ends());
        ends.map(|(a, b)| [a, b].iter().filter_map(|&i| sk.points.get(i).map(|p| p.id.clone())).collect()).unwrap_or_default()
    };
    pair.iter()
        .flat_map(ends)
        .find(|p| crate::construct::sketch_point_position(st, sketch, p).is_some_and(|at| at.dist(corner.point) <= frame_corners::CORNER_TOL))
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
