//! Frame Auto's laying order (#4003, spec A1-A3): the members in one sketch plane that meet form a group, the
//! members on a group's outline are outer, and a fabricator lays them outer first, then inner, squarest first.
//! [`plan`] then decides what happens at each point (spec A4-A6): the corner, the ends cut to fit, the overrides.
//! Pure geometry on straight members: points and directions only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::{PI, TAU};

use solidcore_record::Vec3;
use solidcraft_geom::prelude::*;

use crate::frame_corners::{CORNER_TOL, CornerOverride, CornerType, EQUAL_LENGTH, away};
use crate::{DocError, Result};

/// A member's plane: Some((normal, sketch x)) for a line of a 2D sketch, None otherwise (pairs, 3D-sketch lines).
pub type MemberPlane = Option<(Vec3, Vec3)>;

/// One member's place in the laying order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rank {
    pub member: usize,
    /// Its group's place in the laying order; the members in no group plane share the one after the last.
    pub group: usize,
    pub outer: bool,
    /// An outer member that runs against its line: the outline runs anticlockwise, seen from the sketch's front.
    pub reverse: bool,
}

/// Two unit directions whose cross product is shorter than this are parallel.
const PARALLEL: f64 = 1e-6;

/// One straight member of a group: its nodes are its two ends and the points on its middle where other members end
/// or cross, in order along it.
struct Line {
    member: usize,
    nodes: Vec<usize>,
    dir: Vec3,
    length: f64,
}

struct Group {
    lines: Vec<Line>,
    /// The sketch's x and y axes, from the first-drawn line of the group's plane.
    x: Vec3,
    y: Vec3,
}

/// Every member in laying order: each planar group's members (A2/A3), groups by their longest member (A1),
/// then the members in no group plane, longest first.
pub fn ranks(segments: &[(String, Vec3, Vec3)], planes: &[MemberPlane], drawn: &[usize]) -> Vec<Rank> {
    laying(segments, planes, drawn).0
}

/// [`ranks`], and each laid group's sketch x: a member is in a group when its rank's group indexes these.
fn laying(segments: &[(String, Vec3, Vec3)], planes: &[MemberPlane], drawn: &[usize]) -> (Vec<Rank>, Vec<Vec3>) {
    // Everything is worked in drawing order, so the pick order never matters.
    let key = |m: usize| (drawn.get(m).copied().unwrap_or(m), m);
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by_key(|&m| key(m));
    let length = |m: usize| segments.get(m).map(|(_, a, b)| a.dist(*b)).filter(|l| l.is_finite()).unwrap_or(0.0);

    let mut points = Points::default();
    let mut groups = Vec::new();
    for (normal, x, ms) in plane_sets(segments, planes, &order) {
        let lines = graph(segments, &ms, &mut points);
        let y = normal.cross(x);
        groups.extend(connected(lines).into_iter().map(|lines| Group { lines, x, y }));
    }
    let longest = |g: usize| groups.get(g).map_or(0.0, |g| g.lines.iter().map(|l| l.length).fold(0.0, f64::max));
    let first = |g: usize| groups.get(g).and_then(|g| g.lines.iter().map(|l| key(l.member)).min());
    let laid = longest_first((0..groups.len()).collect(), longest, first);

    let mut out = Vec::with_capacity(segments.len());
    let mut grouped = vec![false; segments.len()];
    for (gi, g) in laid.iter().filter_map(|&g| groups.get(g)).enumerate() {
        for (li, outer, reverse) in group_order(g, &key, &points.at) {
            let Some(line) = g.lines.get(li) else { continue };
            if let Some(seen) = grouped.get_mut(line.member) {
                *seen = true;
            }
            out.push(Rank { member: line.member, group: gi, outer, reverse });
        }
    }
    let rest = order.into_iter().filter(|&m| !grouped.get(m).copied().unwrap_or(false)).collect();
    out.extend(longest_first(rest, length, key).into_iter().map(|m| Rank { member: m, group: laid.len(), outer: false, reverse: false }));
    (out, laid.iter().filter_map(|&g| groups.get(g)).map(|g| g.x).collect())
}

/// The planar members in sets that lie in one plane (whichever way their sketches face), each with the first-drawn
/// member's unit normal and sketch x. A member with a non-finite or degenerate line or plane is in none.
fn plane_sets(segments: &[(String, Vec3, Vec3)], planes: &[MemberPlane], order: &[usize]) -> Vec<(Vec3, Vec3, Vec<usize>)> {
    let mut sets: Vec<(Vec3, Vec3, Vec3, Vec<usize>)> = Vec::new(); // (normal, x, a point on it, members)
    for &m in order {
        let Some((_, a, b)) = segments.get(m) else { continue };
        let Some((n, x)) = planes.get(m).copied().flatten() else { continue };
        let len = a.dist(*b);
        if !a.is_finite() || !b.is_finite() || !len.is_finite() || len <= 2.0 * CORNER_TOL || !x.is_finite() {
            continue;
        }
        let Some(n) = n.normalized() else { continue };
        let on = |pn: Vec3, p0: Vec3| n.cross(pn).len() <= PARALLEL && [a, b].iter().all(|p| (**p - p0).dot(pn).abs() <= CORNER_TOL);
        if let Some(set) = sets.iter_mut().find(|s| on(s.0, s.2)) {
            set.3.push(m);
            continue;
        }
        let Some(x) = (x - n * x.dot(n)).normalized().or_else(|| n.any_perp().normalized()) else { continue };
        sets.push((n, x, *a, vec![m]));
    }
    sets.into_iter().map(|(n, x, _, ms)| (n, x, ms)).collect()
}

/// The points where members meet: points within CORNER_TOL of one already found are that one.
#[derive(Default)]
struct Points {
    at: Vec<Vec3>,
    grid: HashMap<(i64, i64, i64), Vec<usize>>,
}

impl Points {
    fn node(&mut self, p: Vec3) -> usize {
        let cell = |p: Vec3| ((p.x / CORNER_TOL).floor() as i64, (p.y / CORNER_TOL).floor() as i64, (p.z / CORNER_TOL).floor() as i64);
        let (x, y, z) = cell(p);
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                for dz in -1..=1i64 {
                    let mut hits = self.grid.get(&(x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz))).into_iter().flatten();
                    if let Some(&i) = hits.find(|&&i| self.at.get(i).is_some_and(|q| q.dist(p) <= CORNER_TOL)) {
                        return i;
                    }
                }
            }
        }
        self.at.push(p);
        self.grid.entry((x, y, z)).or_default().push(self.at.len() - 1);
        self.at.len() - 1
    }
}

/// The lines of the members `ms` (one plane's, in drawing order), each with its nodes: its ends, the other members'
/// ends on its middle (a touch) and the points where its middle crosses another's.
fn graph(segments: &[(String, Vec3, Vec3)], ms: &[usize], points: &mut Points) -> Vec<Line> {
    let ends: Vec<(usize, Vec3, Vec3)> = ms.iter().filter_map(|&m| segments.get(m).map(|(_, a, b)| (m, *a, *b))).collect();
    let nodes: Vec<(usize, usize)> = ends.iter().map(|&(_, a, b)| (points.node(a), points.node(b))).collect();
    let mut middles: Vec<Vec<(f64, usize)>> = vec![Vec::new(); ends.len()];
    // A point is on a line's middle when within CORNER_TOL of it and not of its ends (as find_corners tests it).
    let on_middle = |p: Vec3, a: Vec3, b: Vec3| p.dist_to_segment(a, b) <= CORNER_TOL && p.dist(a) > CORNER_TOL && p.dist(b) > CORNER_TOL;
    let tol = Vec3::new(CORNER_TOL, CORNER_TOL, CORNER_TOL);
    let apart = |(a, b): (Vec3, Vec3), (c, d): (Vec3, Vec3)| {
        let (lo, hi, lo2, hi2) = (a.min(b) - tol, a.max(b) + tol, c.min(d), c.max(d));
        hi2.x < lo.x || hi2.y < lo.y || hi2.z < lo.z || lo2.x > hi.x || lo2.y > hi.y || lo2.z > hi.z
    };
    // ponytail: every pair of members, O(n²) under MAX_MEMBERS (~0.5M cheap tests); sweep by bounding box if it shows.
    for (i, &(_, a1, b1)) in ends.iter().enumerate() {
        for (j, &(_, a2, b2)) in ends.iter().enumerate().skip(i + 1) {
            if apart((a1, b1), (a2, b2)) {
                continue;
            }
            let (Some(&(na1, nb1)), Some(&(na2, nb2))) = (nodes.get(i), nodes.get(j)) else { continue };
            let (Some(d1), Some(d2)) = ((b1 - a1).normalized(), (b2 - a2).normalized()) else { continue };
            // One's end on the other's middle.
            for (p, n, on, (a, b, d)) in
                [(a2, na2, i, (a1, b1, d1)), (b2, nb2, i, (a1, b1, d1)), (a1, na1, j, (a2, b2, d2)), (b1, nb1, j, (a2, b2, d2))]
            {
                if on_middle(p, a, b)
                    && let Some(mid) = middles.get_mut(on)
                {
                    mid.push(((p - a).dot(d), n));
                }
            }
            // Two middles crossing: where the two lines come closest.
            if d1.cross(d2).len() <= PARALLEL {
                continue;
            }
            let (r, c) = (a1 - a2, d1.dot(d2));
            let (e, f) = (d1.dot(r), d2.dot(r));
            let (s, t) = ((c * f - e) / (1.0 - c * c), (f - c * e) / (1.0 - c * c));
            let (p1, p2) = (a1 + d1 * s, a2 + d2 * t);
            let p = (p1 + p2) * 0.5;
            if p.is_finite() && p1.dist(p2) <= CORNER_TOL && on_middle(p, a1, b1) && on_middle(p, a2, b2) {
                let n = points.node(p);
                for (on, t) in [(i, s), (j, t)] {
                    if let Some(mid) = middles.get_mut(on) {
                        mid.push((t, n));
                    }
                }
            }
        }
    }
    ends.iter()
        .zip(nodes)
        .zip(middles)
        .filter_map(|((&(member, a, b), (na, nb)), mut mid)| {
            mid.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut nodes = vec![na];
            nodes.extend(mid.into_iter().map(|(_, n)| n).filter(|&n| n != na && n != nb));
            nodes.push(nb);
            nodes.dedup();
            Some(Line { member, nodes, dir: (b - a).normalized()?, length: a.dist(b) })
        })
        .collect()
}

/// node → the lines through it or ending at it.
fn node_lines(lines: &[Line]) -> HashMap<usize, Vec<usize>> {
    let mut at: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, l) in lines.iter().enumerate() {
        for &n in &l.nodes {
            at.entry(n).or_default().push(k);
        }
    }
    at
}

/// The lines in groups joined through shared points, each group in drawing order.
fn connected(lines: Vec<Line>) -> Vec<Vec<Line>> {
    let at = node_lines(&lines);
    let mut group = vec![usize::MAX; lines.len()];
    let mut count = 0;
    for start in 0..lines.len() {
        if group.get(start) != Some(&usize::MAX) {
            continue;
        }
        let mut todo = vec![start];
        while let Some(k) = todo.pop() {
            match group.get_mut(k) {
                Some(g) if *g == usize::MAX => *g = count,
                _ => continue,
            }
            let nodes = lines.get(k).map(|l| l.nodes.as_slice()).unwrap_or_default();
            todo.extend(nodes.iter().filter_map(|n| at.get(n)).flatten());
        }
        count += 1;
    }
    let mut out: Vec<Vec<Line>> = (0..count).map(|_| Vec::new()).collect();
    for (l, g) in lines.into_iter().zip(group) {
        if let Some(o) = out.get_mut(g) {
            o.push(l);
        }
    }
    out
}

/// The group's lines in laying order (A2/A3), as (line index, outer, reverse).
fn group_order(g: &Group, key: &impl Fn(usize) -> (usize, usize), at: &[Vec3]) -> Vec<(usize, bool, bool)> {
    let length = |k: usize| g.lines.get(k).map_or(0.0, |l| l.length);
    // Ties: nearer the sketch's X axis, to 0.01°, then drawn first.
    let tie =
        |k: usize| g.lines.get(k).map(|l| ((l.dir.dot(g.y).abs().atan2(l.dir.dot(g.x).abs()).to_degrees() * 100.0).round() as i64, key(l.member)));
    let all: Vec<usize> = (0..g.lines.len()).collect();
    let (walked, bridges) = outline(&g.lines, g.x, g.y, at);
    let order = if walked.iter().all(Option::is_none) {
        longest_first(all, length, tie) // no closed shape: longest first
    } else {
        let is = |v: &[bool], k: usize| v.get(k).copied().unwrap_or(false);
        let outer: Vec<bool> = walked.iter().map(Option::is_some).collect();
        let at_node = node_lines(&g.lines);
        // Inner lines running the full way between two outer ones first, then the other inner lines.
        let (between, inner): (Vec<usize>, Vec<usize>) = all.iter().copied().filter(|&k| !is(&outer, k) && !is(&bridges, k)).partition(|&k| {
            g.lines
                .get(k)
                .is_some_and(|l| [l.nodes.first(), l.nodes.last()].into_iter().all(|n| others(g, &at_node, k, n, Some(&outer)).next().is_some()))
        });
        let mut order = longest_first(all.iter().copied().filter(|&k| is(&outer, k)).collect(), length, tie);
        for (class, among) in [(between, Some(outer.as_slice())), (inner, None)] {
            let mut by: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
            for k in class {
                by.entry(squareness(g, &at_node, k, among)).or_default().push(k);
            }
            by.into_values().for_each(|ks| order.extend(longest_first(ks, length, tie)));
        }
        order.extend(longest_first(all.iter().copied().filter(|&k| is(&bridges, k)).collect(), length, tie));
        order
    };
    order
        .into_iter()
        .map(|k| {
            let w = walked.get(k).copied().flatten();
            (k, w.is_some(), w.unwrap_or(false))
        })
        .collect()
}

/// Longest first; a run of items within EQUAL_LENGTH of the run's longest is ordered by tie instead.
fn longest_first<K: Ord>(mut items: Vec<usize>, length: impl Fn(usize) -> f64, tie: impl Fn(usize) -> K) -> Vec<usize> {
    items.sort_by(|&a, &b| length(b).total_cmp(&length(a)));
    let mut out = Vec::with_capacity(items.len());
    let mut rest = items.as_slice();
    while let Some(&first) = rest.first() {
        let n = rest.iter().take_while(|&&k| length(first) - length(k) <= EQUAL_LENGTH).count().max(1);
        let Some((run, more)) = rest.split_at_checked(n) else { break };
        let mut run = run.to_vec();
        run.sort_by_key(|&k| tie(k));
        out.extend(run);
        rest = more;
    }
    out
}

/// For each line, Some(reverse) when it is outer (spec A2), and whether it is a bridge. The lines are cut at their
/// nodes into pieces, the dangling pieces dropped, and the outside of the rest walked anticlockwise about x × y
/// from the lowest point, taking the most clockwise turn at each point; a line runs the way the walk first passes
/// it. A piece the walk runs out along and back is a bridge's, not the outline's. All None with no closed shape.
fn outline(lines: &[Line], x: Vec3, y: Vec3, at: &[Vec3]) -> (Vec<Option<bool>>, Vec<bool>) {
    let mut walked = vec![None; lines.len()];
    let mut bridges = vec![false; lines.len()];
    let mut pieces: Vec<(usize, usize, usize)> =
        lines.iter().enumerate().flat_map(|(k, l)| l.nodes.windows(2).filter_map(move |w| Some((k, *w.first()?, *w.get(1)?)))).collect();
    loop {
        let mut degree: HashMap<usize, usize> = HashMap::new();
        for &(_, a, b) in &pieces {
            *degree.entry(a).or_default() += 1;
            *degree.entry(b).or_default() += 1;
        }
        let before = pieces.len();
        pieces.retain(|(_, a, b)| [a, b].iter().all(|n| degree.get(n).is_some_and(|&d| d > 1)));
        if pieces.len() == before {
            break;
        }
    }
    // Half-edge 2p runs piece p along its line, 2p + 1 against it: (line, from, to, angle).
    let mut half = Vec::with_capacity(2 * pieces.len());
    for &(k, a, b) in &pieces {
        let d = lines.get(k).map_or(Vec3::ZERO, |l| l.dir);
        half.push((k, a, b, d.dot(y).atan2(d.dot(x))));
        half.push((k, b, a, (-d).dot(y).atan2((-d).dot(x))));
    }
    let mut leave: HashMap<usize, Vec<usize>> = HashMap::new();
    for (h, &(_, from, _, _)) in half.iter().enumerate() {
        leave.entry(from).or_default().push(h);
    }
    let leaving = |n: usize| leave.get(&n).map(Vec::as_slice).unwrap_or_default();
    let angle = |h: usize| half.get(h).map_or(0.0, |e| e.3);
    // How far anticlockwise from the way back h leaves: the smallest is the most clockwise turn.
    let turn = |h: usize, back: f64| {
        let t = (angle(h) - back).rem_euclid(TAU);
        if t <= 0.0 { TAU } else { t }
    };
    let low = |n: usize| at.get(n).map_or((f64::INFINITY, f64::INFINITY), |p| ((p.dot(y) * 1e6).round(), (p.dot(x) * 1e6).round()));
    let lowest = half.iter().map(|e| e.1).min_by(|&a, &b| {
        let (la, lb) = (low(a), low(b));
        la.0.total_cmp(&lb.0).then(la.1.total_cmp(&lb.1)).then(a.cmp(&b))
    });
    // The lowest point is on the outline: the walk starts there heading along +x, as if it came from -x.
    let first = |n: usize| leaving(n).iter().copied().min_by(|&a, &b| turn(a, PI).total_cmp(&turn(b, PI)).then(a.cmp(&b)));
    let Some(start) = lowest.and_then(first) else { return (walked, bridges) };
    let mut passed = vec![false; half.len()];
    let mut walk = Vec::new();
    let mut h = start;
    for _ in 0..half.len() {
        walk.push(h);
        if let Some(p) = passed.get_mut(h) {
            *p = true;
        }
        let back = h ^ 1;
        let Some(&(_, _, to, _)) = half.get(h) else { break };
        let next = leaving(to)
            .iter()
            .copied()
            .min_by(|&a, &b| (a == back).cmp(&(b == back)).then(turn(a, angle(back)).total_cmp(&turn(b, angle(back)))).then(a.cmp(&b)));
        match next {
            Some(n) if n != start => h = n,
            _ => break,
        }
    }
    // Passed both ways: the walk ran out along it and back (A2).
    let both = |h: usize| passed.get(h & !1).copied().unwrap_or(false) && passed.get(h | 1).copied().unwrap_or(false);
    for &h in &walk {
        if let (false, Some(&(k, ..))) = (both(h), half.get(h))
            && let Some(w) = walked.get_mut(k)
        {
            w.get_or_insert(h % 2 == 1);
        }
    }
    for &h in &walk {
        if both(h)
            && let Some(&(k, ..)) = half.get(h)
            && let (Some(b), Some(None)) = (bridges.get_mut(k), walked.get(k))
        {
            *b = true;
        }
    }
    (walked, bridges)
}

/// The group's other lines at node n (only those marked in among, if given).
fn others<'a>(
    g: &'a Group,
    at: &'a HashMap<usize, Vec<usize>>,
    k: usize,
    n: Option<&usize>,
    among: Option<&'a [bool]>,
) -> impl Iterator<Item = &'a Line> {
    let js = n.and_then(|n| at.get(n)).map(Vec::as_slice).unwrap_or_default();
    js.iter().filter(move |&&j| j != k && among.is_none_or(|v| v.get(j).copied().unwrap_or(false))).filter_map(|&j| g.lines.get(j))
}

/// How far line k's worse end is from 90°, each end against the line there (in among, if given) it is nearest to
/// square with, in hundredths of a degree; an end in mid-air is square.
fn squareness(g: &Group, at: &HashMap<usize, Vec<usize>>, k: usize, among: Option<&[bool]>) -> i64 {
    let Some(line) = g.lines.get(k) else { return 0 };
    let off = |o: &Line| 90.0 - line.dir.dot(o.dir).abs().min(1.0).acos().to_degrees();
    let worst = [line.nodes.first(), line.nodes.last()]
        .into_iter()
        .filter_map(|end| others(g, at, k, end, among).map(off).min_by(f64::total_cmp))
        .fold(0.0, f64::max);
    (worst * 100.0).round() as i64
}

// --- The plan (spec A4-A6): at each point the corner, the ends cut to fit, and the overrides ---

/// How a corner is jointed (spec A5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint {
    /// The long tube cut square, the short one cut at the angle and sitting on it.
    AngledButt,
    Mitre,
    /// The long tube runs through, the short one is cut to fit against its side.
    Butt,
    /// Not jointed here: the corner lies inside an earlier Frame Member's tube (the builder's call, never plan's).
    Free,
}

/// Where the two earliest group tubes at a point end and no group tube passes through it (spec A4).
#[derive(Clone, Debug, PartialEq)]
pub struct AutoCorner {
    pub point: Vec3,
    /// (member, at_start) of the long tube's end and the short one's, by length with A3's ties; a Flip already applied.
    pub long: (usize, bool),
    pub short: (usize, bool),
    /// How far the short tube swings from in line with the long one: 0 in line, 90 square.
    pub angle_deg: f64,
    /// The table's joint for the angle; between different sections a butt for a mitre.
    pub joint: Joint,
    /// What builds it: Auto for `joint`, else its override's type.
    pub kind: CornerType,
    /// Its override's Flip, already applied to `long` and `short`.
    pub flip: bool,
}

/// A member's end cut to fit the tubes already at its point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fit {
    pub member: usize,
    pub at_start: bool,
    /// The members to fit against, in laying order.
    pub against: Vec<usize>,
}

/// A corner between two members in no group plane, chained and mitred unless an override says otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainCorner {
    /// (member, at_start) of the earlier member's end in laying order, and the later one's.
    pub a: (usize, bool),
    pub b: (usize, bool),
    /// Its override's type, an Auto override meaning the chain's own joint; Mitre without one.
    pub kind: CornerType,
    /// Its override's Flip.
    pub flip: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub ranks: Vec<Rank>,
    pub corners: Vec<AutoCorner>,
    /// In laying order.
    pub fits: Vec<Fit>,
    /// The corners between members in no group plane.
    pub chained: Vec<ChainCorner>,
}

/// The joint for an angle in degrees from in line (spec A5), compared rounded to 0.01° so a boundary never flips.
pub fn table(angle_deg: f64) -> Joint {
    let a = (angle_deg * 100.0).round() / 100.0;
    if a <= 30.0 {
        Joint::AngledButt
    } else if a < 60.0 {
        Joint::Mitre
    } else if a <= 120.0 {
        Joint::Butt
    } else {
        Joint::Mitre
    }
}

/// An end at a point: (laying place, member, at_start).
type End = (usize, usize, bool);

/// What happens at each point where members meet, wherever their planes (spec A4). With a group tube passing
/// through, each group end there fits it and the earlier ends; else the two earliest group ends make a corner and
/// later ones fit; one end alone is left. Members in no group plane come after: the two earliest of theirs at a
/// point chain (mitred, or as their override says), and any other fits every tube already there. Crossing tubes are left whole (stage 3 splits them).
/// An override matches the point whose ending members are exactly its `members` (decision 3); one that matches no
/// corner fails.
pub fn plan(
    segments: &[(String, Vec3, Vec3)],
    planes: &[MemberPlane],
    drawn: &[usize],
    same_section: &dyn Fn(usize, usize) -> bool,
    overrides: &[CornerOverride],
) -> Result<Plan> {
    let (ranks, xs) = laying(segments, planes, drawn);
    let mut place = vec![usize::MAX; segments.len()];
    for (i, r) in ranks.iter().enumerate() {
        if let Some(p) = place.get_mut(r.member) {
            *p = i;
        }
    }
    let pos = |m: usize| place.get(m).copied().unwrap_or(usize::MAX);
    let grouped = |m: usize| ranks.get(pos(m)).is_some_and(|r| r.group < xs.len());
    let length = |m: usize| segments.get(m).map_or(0.0, |(_, a, b)| a.dist(*b));
    // Every member's line, met in laying order, so the points never depend on the pick order.
    let valid: Vec<usize> = ranks
        .iter()
        .map(|r| r.member)
        .filter(|&m| segments.get(m).is_some_and(|(_, a, b)| a.is_finite() && b.is_finite() && length(m).is_finite() && length(m) > 2.0 * CORNER_TOL))
        .collect();
    let mut points = Points::default();
    let lines = graph(segments, &valid, &mut points);
    // node → the ends there, and the members passing through it.
    let mut ends: BTreeMap<usize, Vec<End>> = BTreeMap::new();
    let mut through: HashMap<usize, Vec<usize>> = HashMap::new();
    for l in &lines {
        let (Some(&first), Some(&last)) = (l.nodes.first(), l.nodes.last()) else { continue };
        if first == last {
            continue;
        }
        ends.entry(first).or_default().push((pos(l.member), l.member, true));
        ends.entry(last).or_default().push((pos(l.member), l.member, false));
        for &n in l.nodes.get(1..l.nodes.len().saturating_sub(1)).unwrap_or_default() {
            through.entry(n).or_default().push(l.member);
        }
    }
    let wanted: Vec<BTreeSet<&str>> = overrides.iter().map(|o| o.members.iter().map(String::as_str).collect()).collect();
    let mut used = vec![false; overrides.len()];
    let key = |m: usize| (drawn.get(m).copied().unwrap_or(m), m);
    let (mut corners, mut fits, mut chained) = (Vec::new(), Vec::new(), Vec::new());
    // End e fits against these members (not itself), in laying order; against nothing it is left alone.
    let fit = |e: &End, against: Vec<usize>, fits: &mut Vec<Fit>| {
        let mut against: Vec<usize> = against.into_iter().filter(|&m| m != e.1).collect();
        against.sort_by_key(|&m| pos(m));
        against.dedup();
        if !against.is_empty() {
            fits.push(Fit { member: e.1, at_start: e.2, against });
        }
    };
    for (n, mut here) in ends {
        here.sort_unstable();
        let whole = through.remove(&n).unwrap_or_default();
        let ids: BTreeSet<&str> = here.iter().filter_map(|e| segments.get(e.1)).map(|s| s.0.as_str()).collect();
        let mut matched = || {
            let o = wanted.iter().position(|w| *w == ids)?;
            if let Some(u) = used.get_mut(o) {
                *u = true;
            }
            overrides.get(o)
        };
        let (group, rest): (Vec<End>, Vec<End>) = here.iter().partition(|e| grouped(e.1));
        let group_whole: Vec<usize> = whole.iter().copied().filter(|&m| grouped(m)).collect();
        let mut done = 0;
        if let ([a, b, ..], true) = (group.as_slice(), group_whole.is_empty()) {
            done = 2;
            // Ties as A3: nearer the earlier one's group's sketch X axis, to 0.01°, then drawn first.
            let x = ranks.get(a.0).and_then(|r| xs.get(r.group)).copied().unwrap_or(Vec3::X);
            let tie = |m: usize| {
                let off = segments.get(m).and_then(|(_, p, q)| (*q - *p).normalized()).map(|d| d.dot(x).abs().min(1.0).acos().to_degrees());
                ((off.unwrap_or(90.0) * 100.0).round() as i64, key(m))
            };
            let a_long = longest_first(vec![a.1, b.1], length, tie).first() == Some(&a.1);
            let o = matched();
            let swap = o.is_some_and(|o| o.flip && o.corner.flips());
            let (long, short) = if a_long != swap { ((a.1, a.2), (b.1, b.2)) } else { ((b.1, b.2), (a.1, a.2)) };
            let angle_deg = match (away(segments, long), away(segments, short)) {
                (Some(l), Some(s)) => (-l).dot(s).clamp(-1.0, 1.0).acos().to_degrees(),
                _ => 0.0,
            };
            let joint = match table(angle_deg) {
                Joint::Mitre if !same_section(long.0, short.0) => Joint::Butt,
                j => j,
            };
            let point = points.at.get(n).copied().unwrap_or(Vec3::ZERO);
            let (kind, flip) = o.map_or((CornerType::Auto, false), |o| (o.corner, o.flip));
            corners.push(AutoCorner { point, long, short, angle_deg, joint, kind, flip });
        } else if group_whole.is_empty() {
            done = group.len(); // one end alone
        }
        for (i, e) in group.iter().enumerate().skip(done) {
            fit(e, group_whole.iter().copied().chain(group.iter().take(i).map(|e| e.1)).collect(), &mut fits);
        }
        let mut done = 0;
        if let [a, b, ..] = rest.as_slice() {
            done = 2;
            // An Auto override at a chain's corner is its own joint, a mitre.
            let (kind, flip) = matched().map_or((CornerType::Mitre, false), |o| (o.corner, o.flip));
            let kind = if kind == CornerType::Auto { CornerType::Mitre } else { kind };
            chained.push(ChainCorner { a: (a.1, a.2), b: (b.1, b.2), kind, flip });
        }
        for (j, e) in rest.iter().enumerate().skip(done) {
            let earlier = rest.iter().take(j).map(|e| e.1);
            fit(e, whole.iter().copied().chain(group.iter().map(|e| e.1)).chain(earlier).collect(), &mut fits);
        }
    }
    if used.contains(&false) {
        return Err(DocError::Invalid("A corner override is not at a corner of the frame".into()));
    }
    fits.sort_by_key(|f| pos(f.member));
    Ok(Plan { ranks, corners, fits, chained })
}

#[cfg(test)]
#[path = "frame_layout_tests.rs"]
mod tests;
