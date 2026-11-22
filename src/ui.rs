//! What Softcore draws itself: the keys panel, the question before leaving, and the note
//! about settings that couldn't be read. All of it is egui in the Rivet look (`rivet-look`),
//! painted in memory by `rivet-kit`, the way Rivet Shell paints its panels. The border colours
//! and corner radius come from the same place, so windows match the apps inside them.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2, ViewportId};
use rivet_kit::soft;
use rivet_look::live::{Choice, LiveLook};
use rivet_look::theme;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

use crate::config::{Action, Config};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    None,
    Keys,
    Quit,
    Errors,
}

/// The colours and radius the compositor draws windows with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub active: [u8; 3],
    pub inactive: [u8; 3],
    pub background: [u8; 3],
    pub radius: i32,
}

pub struct Ui {
    ctx: egui::Context,
    tex: soft::Textures,
    live: LiveLook,
    pub panel: Panel,
    buffer: Option<(MemoryRenderBuffer, (i32, i32))>,
    dirty: bool,
    look: Option<Look>,
}

fn choice(c: &Config) -> Choice {
    Choice { mode: c.theme, accent: c.accent, follow_desktop_colors: c.follow_desktop_colors, text_size: 0.0 }
}

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

impl Ui {
    pub fn new() -> Ui {
        Ui { ctx: egui::Context::default(), tex: soft::Textures::default(), live: LiveLook::new(), panel: Panel::None, buffer: None, dirty: true, look: None }
    }

    pub fn show(&mut self, panel: Panel) {
        if self.panel != panel {
            self.panel = panel;
            self.dirty = true;
        }
    }

    pub fn settings_changed(&mut self, _c: &Config) {
        self.look = None;
        self.dirty = true;
    }

    /// Whether the desktop's colours changed on disk (a new wallpaper, a palette chosen in
    /// Rivet Shell). Called every two seconds.
    pub fn check_look(&mut self) -> bool {
        let new = LiveLook::new();
        if new.palette() == self.live.palette() {
            return false;
        }
        self.live = new;
        self.look = None;
        self.dirty = true;
        true
    }

    fn raw(size: (i32, i32)) -> egui::RawInput {
        let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(size.0 as f32, size.1 as f32))), ..Default::default() };
        if let Some(v) = raw.viewports.get_mut(&ViewportId::ROOT) {
            v.native_pixels_per_point = Some(1.0);
        }
        raw
    }

    /// The window colours now: the Rivet accent for the focused window, the theme's outline
    /// for the rest, unless the settings name colours of their own.
    pub fn look(&mut self, c: &Config) -> Look {
        if let Some(l) = self.look {
            return l;
        }
        let ch = choice(c);
        let ctx = self.ctx.clone();
        let live = &mut self.live;
        let mut out = ctx.run_ui(Self::raw((16, 16)), |ui| live.frame(ui.ctx(), &ch, false));
        self.tex.update(&out.textures_delta);
        out.textures_delta.clear();
        let v = ctx.global_style().visuals.clone();
        let look = Look {
            active: c.border_active.unwrap_or(rgb(self.live.accent(&ch))),
            inactive: c.border_inactive.unwrap_or(rgb(v.widgets.noninteractive.bg_stroke.color)),
            background: c.background.unwrap_or(rgb(v.extreme_bg_color)),
            radius: c.radius.unwrap_or(theme::r_md() as i32),
        };
        self.look = Some(look);
        look
    }

    /// The open panel as a picture and its size, painted again only when it changed.
    pub fn panel_buffer(&mut self, c: &Config, mod_is_alt: bool, screen: (i32, i32)) -> Option<(MemoryRenderBuffer, (i32, i32))> {
        if self.panel == Panel::None {
            self.buffer = None;
            return None;
        }
        let fit = |w: i32, h: i32| (w.min(screen.0 - 40).max(120), h.min(screen.1 - 40).max(80));
        let size = match self.panel {
            Panel::Keys => fit(820, 100 + 29 * key_rows(c).len().div_ceil(2) as i32),
            Panel::Quit => fit(440, 150),
            _ => fit(640, 160 + 24 * c.errors.len().min(12) as i32),
        };
        if !self.dirty
            && let Some((b, s)) = &self.buffer
            && *s == size
        {
            return Some((b.clone(), *s));
        }
        let pixels = self.paint(c, mod_is_alt, size);
        let buffer = MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, size, 1, Transform::Normal, None);
        self.buffer = Some((buffer.clone(), size));
        self.dirty = false;
        Some((buffer, size))
    }

    fn paint(&mut self, c: &Config, mod_is_alt: bool, size: (i32, i32)) -> Vec<u8> {
        let ch = choice(c);
        let ctx = self.ctx.clone();
        let live = &mut self.live;
        let panel = self.panel;
        let mut out = ctx.run_ui(Self::raw(size), |ui| {
            live.frame(ui.ctx(), &ch, false);
            let v = ui.visuals().clone();
            egui::Frame::new().fill(v.window_fill).stroke(v.window_stroke).corner_radius(theme::r_lg()).inner_margin(20.0).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match panel {
                    Panel::Keys => keys(ui, c, mod_is_alt),
                    Panel::Quit => {
                        ui.heading("Leave Softcore?");
                        ui.add_space(6.0);
                        ui.label("Every open window closes with it.");
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            key_cap(ui, "Enter");
                            ui.label("Leave");
                            ui.add_space(16.0);
                            key_cap(ui, "Esc");
                            ui.label("Stay");
                        });
                    }
                    Panel::Errors => {
                        ui.heading("Some settings couldn't be read");
                        ui.add_space(6.0);
                        for e in c.errors.iter().take(12) {
                            ui.add(egui::Label::new(e).truncate());
                        }
                        ui.add_space(8.0);
                        ui.add(egui::Label::new(RichText::new(crate::config::config_file().display().to_string()).weak()).truncate());
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            key_cap(ui, "Esc");
                            ui.weak("Close");
                        });
                    }
                    Panel::None => {}
                }
            });
        });
        self.tex.update(&out.textures_delta);
        out.textures_delta.clear();
        let prims = ctx.tessellate(out.shapes, 1.0);
        let (w, h) = (size.0 as usize, size.1 as usize);
        let mut buf = vec![0u8; w * h * 4];
        soft::draw(&mut buf, w, h, soft::Order::Rgba, &prims, &self.tex, 1.0);
        buf
    }
}

fn key_cap(ui: &mut egui::Ui, text: &str) {
    let v = ui.visuals().clone();
    egui::Frame::new().fill(v.faint_bg_color).stroke(v.widgets.noninteractive.bg_stroke).corner_radius(theme::r_xs()).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).strong());
    });
}

/// The bindings, in two columns. The nine workspace keys are shown as one line each way.
fn keys(ui: &mut egui::Ui, c: &Config, mod_is_alt: bool) {
    ui.heading("Keys");
    ui.add_space(8.0);
    let modname = if mod_is_alt { "Alt" } else { "Super" };
    let rows: Vec<(String, String)> = key_rows(c).into_iter().map(|(k, l)| (k.replace("Mod", modname), l)).collect();
    let half = rows.len().div_ceil(2);
    ui.columns(2, |cols| {
        for (i, col) in cols.iter_mut().enumerate() {
            let part = if i == 0 { &rows[..half] } else { &rows[half..] };
            for (k, label) in part {
                col.horizontal(|ui| {
                    key_cap(ui, k);
                    ui.label(label);
                });
            }
        }
    });
}

/// The lines of the keys panel: the keys as written, and what they do.
fn key_rows(c: &Config) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for b in &c.binds {
        let digit = b.key.len() == 1 && b.key.chars().all(|ch| ch.is_ascii_digit());
        let mut shown = b.shown.clone();
        let label = match (&b.action, digit) {
            (Action::FocusWorkspace(0), true) => {
                shown = shown.replace('1', "1…9");
                "Workspace by number".to_string()
            }
            (Action::MoveToWorkspace { idx: 0, .. }, true) => {
                shown = shown.replace('1', "1…9");
                "Move to a workspace by number".to_string()
            }
            (Action::FocusWorkspace(_) | Action::MoveToWorkspace { .. }, true) => continue,
            (a, _) => a.label(),
        };
        // Media keys explain themselves.
        if b.key.starts_with("xf86") {
            continue;
        }
        rows.push((shown, label));
    }
    rows
}
