//! Pictures of a display or of one window, read back from the GPU: for screenshots, for
//! programs that ask to capture the screen, for screen sharing, and for the small pictures in
//! the sharing picker. One path serves them all.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::image_capture_source::{ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler, OutputCaptureSourceState, ToplevelCaptureSourceHandler, ToplevelCaptureSourceState};
use smithay::wayland::image_copy_capture::{BufferConstraints, CaptureFailureReason, Frame, ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef};
use smithay::wayland::shm::with_buffer_contents_mut;

use crate::layout::WinId;
use crate::render::{self, Elem};
use crate::state::State;

/// What a picture is of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Output(String),
    Window(WinId),
}

/// Pixels as red, green, blue, alpha, top row first.
pub struct Shot {
    pub rgba: Vec<u8>,
    pub w: i32,
    pub h: i32,
}

/// What captures need between frames.
#[derive(Default)]
pub struct Captures {
    /// Frames to draw into, by size, so a stream doesn't make a new one each time.
    textures: HashMap<(i32, i32), GlesTexture>,
    /// Programs capturing the screen, and the frames they are waiting for.
    pub sessions: Vec<Session>,
    pub waiting: Vec<(ImageCaptureSource, bool, Frame)>,
}

impl State {
    /// How big a picture of a source is, in pixels.
    pub fn source_size(&self, source: &Source) -> Option<Size<i32, Physical>> {
        match source {
            Source::Output(name) => self.output_named(name)?.current_mode().map(|m| m.size),
            Source::Window(id) => {
                let win = self.wins.get(id)?;
                let scale = win.output.as_deref().and_then(|n| self.output_named(n)).map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0);
                let size = win.window.geometry().size.to_f64().to_physical(scale).to_i32_round::<i32>();
                (size.w > 0 && size.h > 0).then_some(size)
            }
        }
    }
}

/// Draw a source into memory and read it back.
/// `panels`: with Softcore's own panels (a screenshot shows them; a shared screen doesn't).
pub fn capture(state: &mut State, renderer: &mut GlesRenderer, source: &Source, cursor: bool, panels: bool) -> Option<Shot> {
    let size = state.source_size(source)?;
    let (elements, clear, scale): (Vec<Elem>, [f32; 4], f64) = match source {
        Source::Output(name) => {
            let output = state.output_named(name)?.clone();
            let (e, c) = render::elements(state, renderer, &output, cursor, panels);
            (e, c, output.current_scale().fractional_scale())
        }
        Source::Window(id) => {
            let win = state.wins.get(id)?;
            let scale = win.output.as_deref().and_then(|n| state.output_named(n)).map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0);
            let surface = win.window.toplevel()?.wl_surface().clone();
            let at = Point::from((0, 0)) - win.window.geometry().loc;
            let mut e = Vec::new();
            render::surface_elements(renderer, &surface, at, Scale::from(scale), 1.0, &mut e);
            (e, [0.0, 0.0, 0.0, 1.0], scale)
        }
    };
    if state.captures.textures.len() > 6 {
        state.captures.textures.clear();
    }
    let key = (size.w, size.h);
    if !state.captures.textures.contains_key(&key) {
        let t: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).ok()?;
        state.captures.textures.insert(key, t);
    }
    let texture = state.captures.textures.get_mut(&key)?;
    let mut fb = renderer.bind(texture).ok()?;
    let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
    tracker.render_output(renderer, &mut fb, 0, &elements, clear).ok()?;
    let region = Rectangle::<i32, Buffer>::from_size((size.w, size.h).into());
    let mapping = renderer.copy_framebuffer(&fb, region, Fourcc::Abgr8888).ok()?;
    let mut rgba = renderer.map_texture(&mapping).ok()?.to_vec();
    // Nothing is behind the desktop: a picture of it isn't see-through.
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some(Shot { rgba, w: size.w, h: size.h })
}

pub fn save_png(shot: &Shot, path: &std::path::Path) -> Result<(), String> {
    let img = image::RgbaImage::from_raw(shot.w as u32, shot.h as u32, shot.rgba.clone()).ok_or("the frame was an unexpected size")?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    img.save(path).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

/// After each frame is drawn: the screenshot asked for, the frames capturing programs wait
/// for, and the next frame of every screen being shared.
pub fn after_draw(state: &mut State, renderer: &mut GlesRenderer) {
    if let Some(path) = state.screenshot.take() {
        let shot = state.active_output().map(|o| Source::Output(o.name())).and_then(|s| capture(state, renderer, &s, false, true));
        match shot {
            Some(shot) => {
                if let Err(e) = save_png(&shot, &path) {
                    tracing::warn!("{e}");
                }
            }
            None => tracing::warn!("couldn't take a screenshot"),
        }
    }
    for (source, cursor, frame) in std::mem::take(&mut state.captures.waiting) {
        fill_frame(state, renderer, &source, cursor, frame);
    }
    crate::cast::after_draw(state, renderer);
    crate::picker::after_draw(state, renderer);
}

/// What a capturing program's source is.
fn source_of(state: &State, source: &ImageCaptureSource) -> Option<Source> {
    if let Some(o) = source.user_data().get::<WeakOutput>() {
        return o.upgrade().map(|o| Source::Output(o.name()));
    }
    let handle = source.user_data().get::<ForeignToplevelHandle>()?;
    state.wins.iter().find(|(_, w)| w.foreign.matches(handle)).map(|(id, _)| Source::Window(*id))
}

fn fill_frame(state: &mut State, renderer: &mut GlesRenderer, source: &ImageCaptureSource, cursor: bool, frame: Frame) {
    let Some(shot) = source_of(state, source).and_then(|s| capture(state, renderer, &s, cursor, false)) else {
        frame.fail(CaptureFailureReason::Unknown);
        return;
    };
    let buffer = frame.buffer();
    let written = with_buffer_contents_mut(&buffer, |ptr, len, data| {
        let fits = data.width == shot.w && data.height == shot.h && matches!(data.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888) && (data.offset + data.stride * data.height) as usize <= len;
        if !fits {
            return false;
        }
        // Safety: the rows written are inside the pool, checked above.
        let out = unsafe { std::slice::from_raw_parts_mut(ptr.add(data.offset as usize), (data.stride * data.height) as usize) };
        for y in 0..shot.h as usize {
            let row = &shot.rgba[y * shot.w as usize * 4..(y + 1) * shot.w as usize * 4];
            let dst = &mut out[y * data.stride as usize..y * data.stride as usize + shot.w as usize * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
        }
        true
    });
    match written {
        Ok(true) => frame.success(Transform::Normal, None, monotonic_now()),
        Ok(false) => frame.fail(CaptureFailureReason::BufferConstraints),
        Err(_) => frame.fail(CaptureFailureReason::Unknown),
    }
}

fn monotonic_now() -> std::time::Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

impl State {
    fn constraints(&self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let size = self.source_size(&source_of(self, source)?)?;
        Some(BufferConstraints { size: (size.w, size.h).into(), shm: vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888], dma: None })
    }

    /// A display or window changed size: tell the programs capturing it.
    pub fn update_capture_sizes(&mut self) {
        for s in &self.captures.sessions {
            if let Some(c) = self.constraints(&s.source())
                && s.current_constraints().is_none_or(|old| old.size != c.size)
            {
                s.update_constraints(c);
            }
        }
    }
}

impl ImageCaptureSourceHandler for State {}

impl OutputCaptureSourceHandler for State {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ToplevelCaptureSourceHandler for State {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.toplevel_capture_source_state
    }

    fn toplevel_source_created(&mut self, source: ImageCaptureSource, toplevel: ForeignToplevelHandle) {
        source.user_data().insert_if_missing(|| toplevel);
    }
}

impl ImageCopyCaptureHandler for State {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        self.constraints(source)
    }

    fn new_session(&mut self, session: Session) {
        self.captures.sessions.push(session);
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.captures.sessions.retain(|s| *s != session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        // Filled in when the next frame is drawn.
        self.captures.waiting.push((session.source(), session.draw_cursor(), frame));
        self.queue_redraw();
    }
}
