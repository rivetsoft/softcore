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
    /// Choosing what to share (see picker.rs).
    Share,
}

/// The size of a picture in the sharing picker.
pub const THUMB: (f32, f32) = (168.0, 100.0);

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
    /// Whether the desktop asked for dark when last looked.
    dark: bool,
}

fn choice(c: &Config) -> Choice {
    Choice { mode: match c.theme {
        // Nothing tells Softcore's own panels what the desktop chose, as a window system
        // tells an app: ask the desktop's settings directly.
        crate::config::Theme::System if rivet_look::palette::prefers_dark() => theme::Mode::Dark,
        crate::config::Theme::System => theme::Mode::Light,
        crate::config::Theme::Light => theme::Mode::Light,
        crate::config::Theme::Dark => theme::Mode::Dark,
    }, accent: c.accent, follow_desktop_colors: c.follow_desktop_colors, text_size: 0.0 }
}

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

impl Ui {
    pub fn new() -> Ui {
        Ui { ctx: egui::Context::default(), tex: soft::Textures::default(), live: LiveLook::new(), panel: Panel::None, buffer: None, dirty: true, look: None, dark: rivet_look::palette::prefers_dark() }
    }

    pub fn show(&mut self, panel: Panel) {
        if self.panel != panel {
            self.panel = panel;
            self.dirty = true;
        }
    }

    /// Paint the panel again next frame (what it shows changed).
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// The open panel's size, once it has been painted.
    pub fn panel_size(&self) -> Option<(i32, i32)> {
        self.buffer.as_ref().map(|(_, s)| *s)
    }

    /// A picture for a panel to show.
    pub fn texture(&mut self, name: &str, image: egui::ColorImage) -> egui::TextureHandle {
        self.ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
    }

    pub fn settings_changed(&mut self, _c: &Config) {
        self.look = None;
        self.dirty = true;
    }

    /// Whether the desktop's colours changed on disk (a new wallpaper, a palette chosen in
    /// Rivet Shell, light for dark). Called when one of the files changes.
    pub fn check_look(&mut self) -> bool {
        let new = LiveLook::new();
        let dark = rivet_look::palette::prefers_dark();
        if new.palette() == self.live.palette() && dark == self.dark {
            return false;
        }
        self.live = new;
        self.dark = dark;
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
    pub fn panel_buffer(&mut self, c: &Config, mod_is_alt: bool, screen: (i32, i32), mut picker: Option<&mut crate::picker::Picker>) -> Option<(MemoryRenderBuffer, (i32, i32))> {
        if self.panel == Panel::None {
            self.buffer = None;
            return None;
        }
        let fit = |w: i32, h: i32| (w.min(screen.0 - 40).max(120), h.min(screen.1 - 40).max(80));
        let size = match self.panel {
            Panel::Keys => fit(820, 84 + 28 * key_rows(c).len().div_ceil(2) as i32),
            Panel::Quit => fit(440, 150),
            Panel::Share => {
                let n = picker.as_ref().map_or(1, |p| p.items.len()) as i32;
                let per_row = n.clamp(2, 4);
                fit(40 + per_row * (THUMB.0 as i32 + 28), 150 + ((n + per_row - 1) / per_row) * (THUMB.1 as i32 + 78))
            }
            _ => fit(640, 160 + 24 * c.errors.len().min(12) as i32),
        };
        if !self.dirty
            && let Some((b, s)) = &self.buffer
            && *s == size
        {
            return Some((b.clone(), *s));
        }
        let pixels = self.paint(c, mod_is_alt, size, picker.as_deref_mut());
        let buffer = MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, size, 1, Transform::Normal, None);
        self.buffer = Some((buffer.clone(), size));
        self.dirty = false;
        Some((buffer, size))
    }

    fn paint(&mut self, c: &Config, mod_is_alt: bool, size: (i32, i32), mut picker: Option<&mut crate::picker::Picker>) -> Vec<u8> {
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
                    Panel::Share => {
                        if let Some(p) = picker.as_deref_mut() {
                            share(ui, p);
                        }
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

/// The sharing picker: a card for each display and window, the chosen one outlined.
fn share(ui: &mut egui::Ui, p: &mut crate::picker::Picker) {
    ui.heading(if p.app.is_empty() { "Share your screen".to_string() } else { format!("{} wants to share your screen", p.app) });
    ui.add_space(4.0);
    ui.weak("Choose a display or a window. It is shared until the app stops, or you close it.");
    ui.add_space(12.0);
    let v = ui.visuals().clone();
    let mut rects = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        for (i, item) in p.items.iter().enumerate() {
            let chosen = i == p.selected;
            let frame = egui::Frame::new()
                .fill(if chosen { v.selection.bg_fill.gamma_multiply(0.35) } else { v.faint_bg_color })
                .stroke(if chosen { egui::Stroke::new(2.0, v.selection.bg_fill) } else { v.widgets.noninteractive.bg_stroke })
                .corner_radius(theme::r_md())
                .inner_margin(8.0);
            let shown = frame.show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(THUMB.0);
                    let (area, _) = ui.allocate_exact_size(egui::vec2(THUMB.0, THUMB.1), egui::Sense::hover());
                    ui.painter().rect_filled(area, theme::r_sm(), v.extreme_bg_color);
                    if let Some(t) = &item.thumb {
                        // The picture keeps its shape, in the middle of its place.
                        let size = t.size_vec2();
                        let k = (area.width() / size.x).min(area.height() / size.y);
                        let fitted = Rect::from_center_size(area.center(), size * k);
                        egui::Image::new(egui::load::SizedTexture::new(t.id(), fitted.size())).paint_at(ui, fitted);
                    }
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(&item.title).strong()).truncate());
                    ui.add(egui::Label::new(RichText::new(&item.detail).weak()).truncate());
                });
            });
            rects.push(shown.response.rect);
        }
    });
    p.rects = rects;
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        key_cap(ui, "Enter");
        ui.label("Share");
        ui.add_space(16.0);
        key_cap(ui, "Esc");
        ui.label("Cancel");
    });
}
