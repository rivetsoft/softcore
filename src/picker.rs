//! Choosing what to share: a panel Softcore draws itself, in the Rivet look, listing the
//! displays and windows with a small picture of each. Because the compositor draws it, no
//! program can cover it, imitate it or answer it for you.
//!
//! Whoever asks (`softcore msg pick-cast`, or the portal for an app) is told the choice, or
//! that there was none.

use std::io::Write;
use std::os::unix::net::UnixStream;

use eframe::egui;
use serde_json::json;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point};

use crate::capture::{Source, capture};
use crate::state::State;
use crate::ui::Panel;

pub struct Item {
    pub source: Source,
    pub title: String,
    /// "Display", or the window's app.
    pub detail: String,
    pub thumb: Option<egui::TextureHandle>,
}

pub struct Picker {
    /// The app asking, when it said.
    pub app: String,
    pub items: Vec<Item>,
    pub selected: usize,
    /// Where each item was last drawn, in the panel's own coordinates.
    pub rects: Vec<egui::Rect>,
    owner: UnixStream,
    thumbs_wanted: bool,
}

/// Show the picker. `displays`/`windows`: which kinds the asker can take.
pub fn open(state: &mut State, displays: bool, windows: bool, app: String, mut owner: UnixStream) {
    // One question at a time: a second asker is told no, the first keeps its picker.
    if state.picker.is_some() {
        let _ = writeln!(owner, "{}", json!({"error": "something else is already asking what to share"}));
        return;
    }
    let mut items = Vec::new();
    if displays {
        for o in &state.outputs {
            let props = o.physical_properties();
            let name = if props.make == "Softcore" || props.model == "Unknown" { o.name() } else { format!("{} {}", props.make, props.model) };
            items.push(Item { source: Source::Output(o.name()), title: name, detail: format!("Display {}", o.name()), thumb: None });
        }
    }
    if windows {
        let mut ids: Vec<_> = state.wins.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let (title, app_id) = State::title_and_app_id(&state.wins[&id].window);
            items.push(Item { source: Source::Window(id), title: if title.is_empty() { app_id.clone() } else { title }, detail: app_id, thumb: None });
        }
    }
    if items.is_empty() {
        let _ = writeln!(owner, "{}", json!({"error": "there is nothing to share"}));
        return;
    }
    let _ = owner.set_nonblocking(true);
    state.picker = Some(Picker { app, items, selected: 0, rects: Vec::new(), owner, thumbs_wanted: true });
    state.ui.show(Panel::Share);
    state.ui.touch();
    state.queue_redraw();
}

/// Answer the asker and put the picker away. `choice` None: nothing was chosen.
pub fn finish(state: &mut State, choice: Option<usize>) {
    let Some(mut p) = state.picker.take() else { return };
    let reply = match choice.and_then(|i| p.items.get(i)) {
        Some(Item { source: Source::Output(name), .. }) => json!({"ok": {"kind": "output", "target": name}}),
        Some(Item { source: Source::Window(id), .. }) => json!({"ok": {"kind": "window", "target": id}}),
        None => json!({"error": "nothing was chosen"}),
    };
    let _ = writeln!(p.owner, "{reply}");
    state.ui.show(Panel::None);
    state.queue_redraw();
}

pub fn select(state: &mut State, by: i32) {
    if let Some(p) = &mut state.picker {
        let n = p.items.len() as i32;
        p.selected = (p.selected as i32 + by).rem_euclid(n) as usize;
        state.ui.touch();
        state.queue_redraw();
    }
}

/// The item under a point of the screen, if the picker is open and the point is on one.
pub fn item_at(state: &State, pos: Point<f64, Logical>) -> Option<usize> {
    let p = state.picker.as_ref()?;
    let output = state.active_output()?;
    let geo = State::output_geometry(output);
    let (w, h) = state.ui.panel_size()?;
    let local = pos - geo.loc.to_f64() - Point::from((((geo.size.w - w) / 2) as f64, ((geo.size.h - h) / 2) as f64));
    p.rects.iter().position(|r| r.contains(egui::pos2(local.x as f32, local.y as f32)))
}

/// The pointer moved: the item under it is the selected one.
pub fn hover(state: &mut State, pos: Point<f64, Logical>) {
    if let Some(i) = item_at(state, pos)
        && state.picker.as_ref().is_some_and(|p| p.selected != i)
    {
        state.picker.as_mut().unwrap().selected = i;
        state.ui.touch();
        state.queue_redraw();
    }
}

/// Once round the loop: an asker that left takes its question with it, and so does a picker
/// whose every choice has gone.
pub fn refresh(state: &mut State) {
    let Some(p) = &mut state.picker else { return };
    let mut byte = [0u8; 16];
    let gone = match std::io::Read::read(&mut p.owner, &mut byte) {
        Ok(0) => true,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
        Ok(_) => false,
    };
    if gone {
        finish(state, None);
        return;
    }
    let before = p.items.len();
    let (wins, outputs) = (&state.wins, &state.outputs);
    p.items.retain(|i| match &i.source {
        Source::Output(n) => outputs.iter().any(|o| o.name() == *n),
        Source::Window(id) => wins.contains_key(id),
    });
    if p.items.is_empty() {
        finish(state, None);
    } else if p.items.len() != before {
        p.selected = p.selected.min(p.items.len() - 1);
        state.ui.touch();
        state.queue_redraw();
    }
}

/// After a frame: take the small pictures, once, the first time the picker is shown.
pub fn after_draw(state: &mut State, renderer: &mut GlesRenderer) {
    if !state.picker.as_ref().is_some_and(|p| p.thumbs_wanted) {
        return;
    }
    let sources: Vec<Source> = state.picker.as_ref().unwrap().items.iter().map(|i| i.source.clone()).collect();
    let mut thumbs = Vec::new();
    for (n, source) in sources.iter().enumerate() {
        let thumb = capture(state, renderer, source, false, false).and_then(|shot| {
            let img = image::RgbaImage::from_raw(shot.w as u32, shot.h as u32, shot.rgba)?;
            let small = image::imageops::thumbnail(&img, 2 * crate::ui::THUMB.0 as u32, 2 * crate::ui::THUMB.1 as u32);
            let small = fit(&img, small);
            let color = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
            Some(state.ui.texture(&format!("share-{n}"), color))
        });
        thumbs.push(thumb);
    }
    if let Some(p) = &mut state.picker {
        for (item, thumb) in p.items.iter_mut().zip(thumbs) {
            item.thumb = thumb;
        }
        p.thumbs_wanted = false;
    }
    state.ui.touch();
    state.queue_redraw();
}

/// A thumbnail that keeps the picture's shape (the first try may have squashed it).
fn fit(img: &image::RgbaImage, squashed: image::RgbaImage) -> image::RgbaImage {
    let (bw, bh) = (2.0 * crate::ui::THUMB.0, 2.0 * crate::ui::THUMB.1);
    let k = (bw / img.width() as f32).min(bh / img.height() as f32);
    let (w, h) = ((img.width() as f32 * k).round().max(1.0) as u32, (img.height() as f32 * k).round().max(1.0) as u32);
    if (w, h) == squashed.dimensions() { squashed } else { image::imageops::thumbnail(img, w, h) }
}
