use solidcraft_engine::panels::*;

use super::drag_paint::WindowCard;
use super::workspace::WindowDragEvent;
use super::{ContentServices, ContentSlot, FramePanelContent, PanelContext, PanelController, PanelHost, PanelPresentation, PanelRuntime};
use crate::ui_kit::{self, BackingTooltipExt as _, Container, Edge, HostChrome, Surface};
use crate::window_chrome::TitleBar;
use solidcraft_theme::Tokens;

pub(super) enum WindowAction {
    Activate(InstanceId),
    Expand(SectionId),
    Content(InstanceId, ControlId),
    Redock,
    /// The grip is dragging the active panel back towards the app.
    Drag(WindowDragEvent),
}

/// `title_bar`: the window shows the app's own title bar, whose grip drags the
/// panel back into the app.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    runtime: &mut PanelRuntime,
    services: &mut ContentServices<'_>,
    content: &mut dyn FramePanelContent,
    host: &HostId,
    section: &Section,
    ui: &mut egui::Ui,
    enabled: bool,
    title_bar: Option<TitleBar>,
) -> PanelResult<Option<WindowAction>> {
    if ui.input(|input| input.viewport().close_requested()) {
        return Ok(Some(WindowAction::Redock));
    }
    if ui.input(|input| input.viewport().visible() == Some(false)) {
        runtime.mount_native(host, &ResolvedLayout::default(), &mut super::workspace::EmptyPanelHost, services)?;
        return Ok(None);
    }
    let outer = super::workspace::logical_rect(ui.max_rect());
    let mut action = None;
    if let Some(bar) = title_bar {
        let availability = section.tabs.iter().map(|id| (id.clone(), runtime.layout().availability(id))).collect();
        if let Some(active) = presented_tab(section, &availability) {
            let title = runtime.instance_presentation(active)?.title;
            let rtl = runtime.environment().direction == Direction::Rtl;
            let (drag, caption) = window_bar(ui, bar, host, active, &title, rtl, enabled);
            action = drag.map(WindowAction::Drag);
            if let Some(caption) = caption {
                caption.send(ui.ctx());
            }
        }
        if bar.caption_buttons {
            crate::window_chrome::resize_zones(ui);
        }
    }
    let panel = Container::new(ui.ctx(), Surface::Viewport).central();
    let frame = Container::new(ui.ctx(), Surface::Panel).edge_frame(solidcraft_theme::Tokens::get().dialog_body);
    Container::show_central(ui, panel, frame, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let availability = section.tabs.iter().map(|id| (id.clone(), runtime.layout().availability(id))).collect();
            let Some(active) = presented_tab(section, &availability) else {
                return runtime.mount_native(host, &ResolvedLayout::default(), &mut super::workspace::EmptyPanelHost, services);
            };
            let presentation = runtime.instance_presentation(active)?;
            let layout = runtime.layout();
            let spec = layout.registry().get(&layout.layout().instance(active)?.kind)?;
            let environment = runtime.environment();
            let tabs = section.tabs.iter().filter(|id| availability.get(*id) == Some(&Availability::Available));
            if section.tabs.len() > 1 || section.collapsed || !presentation.actions.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    if section.tabs.len() > 1 {
                        for id in tabs {
                            if let Ok(tab) = runtime.instance_presentation(id)
                                && ui_kit::choice_chip(ui, &id.0, &tab.title, id == active).clicked()
                            {
                                action = Some(WindowAction::Activate(id.clone()));
                            }
                        }
                    }
                    let size = egui::Vec2::splat(Tokens::get().form_close_size);
                    if section.collapsed {
                        let (id, bounds) = ui.allocate_space(size);
                        let label = solidcraft_ui_text::tr("ui.panel_host.expand");
                        let response = ui_kit::panel_host_disclosure(ui, id, bounds, label, true, true, environment.motion == super::Motion::Reduced);
                        if response.clicked() {
                            action = Some(WindowAction::Expand(section.id.clone()));
                        }
                    }
                    for declared in &spec.header_actions {
                        let Some(entry) = presentation.actions.iter().find(|entry| entry.id == declared.id) else { continue };
                        let (_, bounds) = ui.allocate_space(size);
                        let id = ui.make_persistent_id((active, &entry.id));
                        let response = ui_kit::panel_host_control(
                            ui,
                            id,
                            bounds,
                            HostChrome {
                                label: &entry.label,
                                icon: Some(&declared.icon),
                                icon_only: true,
                                attention: false,
                                selected: entry.pressed.unwrap_or(false),
                                enabled: entry.enabled,
                                drag: false,
                                title: false,
                                grip_only: false,
                                rtl: environment.direction == Direction::Rtl,
                            },
                        );
                        if let Some(pressed) = entry.pressed {
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_toggled(if pressed { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False });
                            });
                        }
                        if let Some(reason) = &entry.disabled_reason {
                            response.clone().backing_on_hover_text(reason);
                        }
                        if response.clicked() {
                            action = Some(WindowAction::Content(active.clone(), entry.id.clone()));
                        }
                    }
                });
            }
            let body = ui.available_rect_before_wrap();
            let (min, max) = tab_bounds(runtime.layout().layout(), runtime.layout().registry(), section)?;
            if !outer.size.within(min, max) || !body.is_positive() {
                return runtime.mount_native(host, &ResolvedLayout::default(), &mut super::workspace::EmptyPanelHost, services);
            }
            let geometry = ResolvedPanel {
                instance: active.clone(),
                section: section.id.clone(),
                zone: Zone::Window,
                outer,
                body: (!section.collapsed).then(|| super::workspace::logical_rect(body)),
                painted: vec![outer],
                input: Vec::new(),
            };
            runtime.mount_native(host, &ResolvedLayout { panels: vec![geometry], ..Default::default() }, &mut NativeContent { ui, content }, services)
        })
        .inner
    })
    .inner?;
    Ok(action)
}

/// The window a drag shows before the panel opens in it: the same title bar,
/// and the panel's body as painted this frame where its content will be.
pub(super) fn paint_drop(ui: &mut egui::Ui, title_bar: Option<TitleBar>, active: &InstanceId, title: &str, rtl: bool, card: Option<&WindowCard>) {
    if let Some(bar) = title_bar {
        window_bar(ui, bar, "drop", active, title, rtl, true);
    }
    let panel = Container::new(ui.ctx(), Surface::Viewport).central();
    let frame = Container::new(ui.ctx(), Surface::Panel).edge_frame(Tokens::get().dialog_body);
    Container::show_central(ui, panel, frame, |ui| {
        if let Some(card) = card {
            card.paint(ui, ui.max_rect());
        }
    });
}

/// The window's own title bar, as the app window has: the OS's window controls,
/// and the panel's title with its grip, which drags the panel back over the app.
fn window_bar(
    ui: &mut egui::Ui,
    bar: TitleBar,
    id: impl std::hash::Hash + std::fmt::Debug,
    active: &InstanceId,
    title: &str,
    rtl: bool,
    enabled: bool,
) -> (Option<WindowDragEvent>, Option<crate::window_chrome::WindowAction>) {
    let edge = Container::new(ui.ctx(), Surface::ApplicationBar).edge(("sc_panel_window_bar", id), Edge::Top);
    let frame = Container::new(ui.ctx(), Surface::ApplicationBar).edge_frame(Tokens::get().app_bar);
    let height = crate::toolbar::APP_BAR_HEIGHT;
    Container::new(ui.ctx(), Surface::ApplicationBar)
        .show_edge(ui, edge.exact_size(height).show_separator_line(false), frame, |ui| {
            if !enabled {
                ui.disable();
            }
            let mut drag = None;
            let rect = ui.max_rect();
            let caption = bar.show(ui, rect, |ui, content| drag = title_grip(ui, content, active, title, rtl));
            (drag, caption)
        })
        .inner
}

/// The panel's title with the section grip a docked header has, here dragging
/// the panel back over the app. Positions are screen points; Escape cancels.
fn title_grip(ui: &mut egui::Ui, content: egui::Rect, active: &InstanceId, title: &str, rtl: bool) -> Option<WindowDragEvent> {
    let t = Tokens::get();
    let text = ui.painter().layout_no_wrap(title.to_owned(), ui_kit::type_face::semibold(t.font_heading), t.form_ink).size().x;
    // As the kit lays out a title control: insets, the grip and its gap, the text.
    let width = (2.0 * t.space_12 + t.space_10 + t.space_8 + text).min(content.width());
    let x = if rtl { content.right() - width } else { content.left() };
    let bounds = egui::Rect::from_min_size(egui::pos2(x, content.top()), egui::vec2(width, content.height()));
    let id = ui.make_persistent_id(("panel-window-title", active));
    let chrome = HostChrome {
        label: title,
        icon: None,
        icon_only: false,
        attention: false,
        selected: false,
        enabled: true,
        drag: true,
        title: true,
        grip_only: false,
        rtl,
    };
    let response = ui_kit::panel_host_control(ui, id, bounds, chrome);
    response.clone().backing_on_hover_text(solidcraft_ui_text::tr("ui.panel_host.dock_grip"));
    let at = ui.input(|input| {
        let pointer = input.pointer.latest_pos()?;
        let origin = input.viewport().inner_rect?.min;
        Some(Point { x: origin.x + pointer.x, y: origin.y + pointer.y }).filter(|at| at.validate().is_ok())
    });
    if response.drag_started() {
        return at.map(|at| WindowDragEvent::Begin { instance: active.clone(), at });
    }
    if response.drag_stopped() {
        return Some(at.map_or(WindowDragEvent::Cancel, |at| WindowDragEvent::Release { at }));
    }
    if response.dragged() {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            return Some(WindowDragEvent::Cancel);
        }
        return at.map(|at| WindowDragEvent::Move { at });
    }
    None
}

struct NativeContent<'a> {
    ui: &'a mut egui::Ui,
    content: &'a mut dyn FramePanelContent,
}

impl PanelHost for NativeContent<'_> {
    fn mount(
        &mut self,
        geometry: &ResolvedPanel,
        _presentation: &PanelPresentation,
        context: &PanelContext,
        controller: &mut dyn PanelController,
        services: &mut ContentServices<'_>,
    ) -> PanelResult<()> {
        if let Some(body) = geometry.body {
            let rect = super::workspace::egui_rect(body);
            let mut ui = self.ui.new_child(egui::UiBuilder::new().id_salt(&geometry.instance).max_rect(rect));
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
            self.content.render(ContentSlot::Body, &mut ui, context, controller, services)?;
        }
        Ok(())
    }
}
