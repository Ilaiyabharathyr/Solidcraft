//! Picking and drawing numbered copies while editing a pattern's skip list.

use std::collections::BTreeSet;

use egui::{FontId, Modifiers, Pos2, Rect, Stroke, Vec2};
use serde_json::Value;
use solidcraft_engine::doc::{PatternMarker, PatternMember};
use solidcraft_engine::geom::prelude::*;
use solidcraft_theme::Tokens;

use crate::{SolveApp, dialogs::Dialog, selection::BoxSel, viewport::Proj};

const MARKER_RADIUS: f32 = 7.0;

pub(crate) struct Cache {
    session: u64,
    revision: u64,
    component: u64,
    frame: [[f64; 4]; 4],
    command: &'static str,
    params: Value,
    markers: Vec<PatternMarker>,
}

fn cache_for_session(cache: &Option<Cache>, session: u64) -> Option<&Cache> {
    cache.as_ref().filter(|c| c.session == session)
}

pub(crate) fn active(app: &SolveApp) -> bool {
    app.dialog.as_ref().is_some_and(Dialog::pattern_skip_active)
}

pub(crate) fn project(app: &mut SolveApp, proj: &Proj) -> Vec<(usize, Pos2)> {
    let Some((command, params)) =
        app.dialog.as_ref().filter(|d| d.pattern_skip_active() || d.has_pattern_skips()).and_then(|d| crate::dialogs::pattern_skip_params(app, d))
    else {
        app.viewport.pattern_skip_cache = None;
        return Vec::new();
    };
    let revision = app.session.revision;
    let session = &app.session;
    let component =
        app.dialog.as_ref().and_then(|d| d.editing).and_then(|(id, _)| session.doc.feature(id)).map_or(session.active_component, |f| f.component);
    let frame = crate::dialogs::feature_frame(session, component);
    if cache_for_session(&app.viewport.pattern_skip_cache, session.uid)
        .is_none_or(|c| c.revision != revision || c.component != component || c.command != command || c.params != params || c.frame != frame)
    {
        let markers = if component == session.active_component {
            session.pattern_markers(command, &params)
        } else {
            let mut author = session.scratch();
            author.active_component = component;
            author.active_occurrence_path = None;
            author.pattern_markers(command, &params)
        }
        .unwrap_or_default();
        app.viewport.pattern_skip_cache = Some(Cache { session: session.uid, revision, component, frame, command, params, markers });
    }
    if !active(app) {
        return Vec::new();
    }
    let back = proj.cam.back();
    let eye = proj.cam.eye();
    app.viewport
        .pattern_skip_cache
        .iter()
        .flat_map(|c| c.markers.iter())
        .enumerate()
        .filter_map(|(index, marker)| {
            let [first, second] = marker.positions;
            let towards_eye = if proj.cam.fov > 0.0 { eye - (first + second) * 0.5 } else { back };
            let world = if (second - first).dot(towards_eye) > 0.0 { second } else { first };
            let point = proj.to_screen(world)?;
            (point.is_finite() && proj.rect.contains(point) && !crate::shell::covered(app, point)).then_some((index + 2, point))
        })
        .collect()
}

pub(crate) fn skipped_numbers(app: &SolveApp, dialog: &Dialog) -> Vec<usize> {
    let (slots, members) = dialog.pattern_skipped();
    let Some(cache) = cache_for_session(&app.viewport.pattern_skip_cache, app.session.uid) else { return Vec::new() };
    cache.markers.iter().enumerate().filter_map(|(i, marker)| is_skipped(marker, slots, members).then_some(i + 2)).collect()
}

fn skipped_slot(marker: &PatternMarker, slots: &[usize]) -> Option<usize> {
    marker.legacy_slot.filter(|slot| slots.binary_search(slot).is_ok())
}

fn is_skipped(marker: &PatternMarker, slots: &[usize], members: &[PatternMember]) -> bool {
    skipped_slot(marker, slots).is_some() || members.binary_search(&marker.member).is_ok() || members.binary_search(&marker.legacy_member).is_ok()
}

pub(crate) fn paint(app: &SolveApp, painter: &egui::Painter, markers: &[(usize, Pos2)]) {
    let Some(dialog) = app.dialog.as_ref() else { return };
    let skipped = skipped_numbers(app, dialog);
    let t = Tokens::view();
    let font = FontId::monospace(t.font_small);
    for &(number, at) in markers {
        let label = painter.layout_no_wrap(number.to_string(), font.clone(), t.nav_tip_ink);
        let label_at = at + Vec2::new(MARKER_RADIUS + t.space_4, -label.size().y * 0.5);
        let badge = Rect::from_min_size(label_at, label.size()).expand2(Vec2::new(t.space_2, 0.0));
        painter.rect_filled(badge, t.radius_sm, t.nav_tip);
        painter.galley(label_at, label, t.nav_tip_ink);
        crate::scenario::publish_handle(&format!("pattern:label:{number}"), badge.center());
    }
    // Leave the marker itself visible and clickable if a nearby marker's label overlaps it.
    for &(number, at) in markers {
        let skip = skipped.binary_search(&number).is_ok();
        let fill = if skip { t.error } else { t.agent_ok };
        painter.circle(at, MARKER_RADIUS, fill, Stroke::new(1.5, t.body_edge));
        if skip {
            painter.line_segment([at - Vec2::new(3.5, 0.0), at + Vec2::new(3.5, 0.0)], Stroke::new(1.6, t.body_edge));
        }
        crate::scenario::publish_handle(&format!("pattern:instance:{number}"), at);
    }
}

pub(crate) fn click(app: &mut SolveApp, markers: &[(usize, Pos2)], at: Pos2, modifiers: Modifiers) {
    let closest =
        markers.iter().filter(|(_, p)| p.distance(at) <= MARKER_RADIUS + 3.0).min_by(|(_, a), (_, b)| a.distance(at).total_cmp(&b.distance(at)));
    if let Some(&(number, _)) = closest {
        select(app, std::iter::once(number), modifiers);
    }
}

pub(crate) fn window(app: &mut SolveApp, markers: &[(usize, Pos2)], mut region: BoxSel, modifiers: Modifiers) {
    region.crossing = false;
    let enclosed = markers.iter().filter_map(|&(number, p)| {
        let corners = Rect::from_center_size(p, Vec2::splat(2.0 * MARKER_RADIUS));
        region.takes(&[corners.left_top(), corners.right_top(), corners.right_bottom(), corners.left_bottom()]).then_some(number)
    });
    select(app, enclosed, modifiers);
}

fn select(app: &mut SolveApp, numbers: impl Iterator<Item = usize>, modifiers: Modifiers) {
    // OK has already captured the commands while its result is computing.
    if app.applying.is_some() {
        return;
    }
    let Some(cache) = cache_for_session(&app.viewport.pattern_skip_cache, app.session.uid) else { return };
    let Some(dialog) = app.dialog.as_mut() else { return };
    apply_selection(cache, dialog, numbers, modifiers);
}

pub(crate) fn restore(app: &SolveApp, dialog: &mut Dialog, number: usize) {
    if let Some(cache) = cache_for_session(&app.viewport.pattern_skip_cache, app.session.uid) {
        apply_selection(cache, dialog, std::iter::once(number), Modifiers::ALT);
    }
}

fn apply_selection(cache: &Cache, dialog: &mut Dialog, numbers: impl Iterator<Item = usize>, modifiers: Modifiers) {
    let (slots, members) = dialog.pattern_skipped();
    let mut picked = numbers
        .filter_map(|n| cache.markers.get(n.checked_sub(2)?))
        .filter(|marker| {
            let skipped = is_skipped(marker, slots, members);
            if modifiers.alt { skipped } else { !modifiers.shift || !skipped }
        })
        .peekable();
    if picked.peek().is_none() {
        return;
    }
    let mut members: BTreeSet<_> = members.iter().cloned().collect();
    let mut remaining: BTreeSet<_> = slots.iter().copied().collect();
    // Expand whole placements and translate numbered members from old designs.
    // Skips with no marker in the current layout stay saved.
    for marker in &cache.markers {
        if let Some(slot) = skipped_slot(marker, slots) {
            members.insert(marker.member.clone());
            remaining.remove(&slot);
        }
        if marker.legacy_member != marker.member && members.remove(&marker.legacy_member) {
            members.insert(marker.member.clone());
        }
    }
    for marker in picked {
        if modifiers.alt || (!modifiers.shift && members.contains(&marker.member)) {
            members.remove(&marker.member);
        } else {
            members.insert(marker.member.clone());
        }
    }
    dialog.set_pattern_skipped(remaining.into_iter().collect(), members.into_iter().collect());
}
