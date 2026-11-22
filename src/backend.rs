//! Where Softcore's picture goes: a window on another desktop (to try it out), or nowhere at
//! all (`--headless`: a display in memory, for checks and screenshots that never wait for a
//! screen). Both draw with the same renderer and the same frame.

use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::{GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::{Bind, ImportDma, Offscreen};
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Transform;

use crate::render;
use crate::state::State;

pub enum Backend {
    Winit(Winit),
    Headless(Headless),
    Udev(Box<crate::udev::Udev>),
}

impl Backend {
    pub fn renderer(&mut self) -> &mut GlesRenderer {
        match self {
            Backend::Winit(w) => w.backend.renderer(),
            Backend::Headless(h) => &mut h.renderer,
            Backend::Udev(u) => &mut u.renderer,
        }
    }

    /// Switch to another TTY (only a real session has any).
    pub fn change_vt(&mut self, vt: i32) {
        if let Backend::Udev(u) = self {
            u.change_vt(vt);
        }
    }

    fn draw(&mut self, state: &mut State) {
        match self {
            Backend::Winit(w) => w.draw(state),
            Backend::Headless(h) => h.draw(state),
            Backend::Udev(u) => u.draw(state),
        }
    }
}

pub struct Winit {
    backend: WinitGraphicsBackend<GlesRenderer>,
    /// Whether swapping has been told not to wait for the desktop outside.
    unthrottled: bool,
    output: Output,
    tracker: OutputDamageTracker,
}

pub struct Headless {
    renderer: GlesRenderer,
    target: GlesRenderbuffer,
    output: Output,
    tracker: OutputDamageTracker,
}

fn new_output(name: &str, model: &str) -> Output {
    Output::new(name.to_string(), PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "Softcore".into(), model: model.into(), serial_number: "Unknown".into() })
}

/// What both need once there is a renderer: the shaders, and buffers straight from the GPU.
pub fn renderer_ready(state: &mut State, renderer: &mut GlesRenderer) {
    state.shaders = render::Shaders::compile(renderer);
    let formats = renderer.dmabuf_formats();
    if formats.iter().next().is_some() {
        state.dmabuf_global = Some(state.dmabuf_state.create_global::<State>(&state.dh, formats));
    }
}

/// Draw when something changed, sixty times a second at most.
pub fn start_frames(event_loop: &EventLoop<'static, State>) -> Result<(), String> {
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, state| {
            if state.advance()
                && let Some(mut backend) = state.backend.take()
            {
                backend.draw(state);
                state.backend = Some(backend);
            }
            TimeoutAction::ToDuration(Duration::from_millis(16))
        })
        .map(|_| ())
        .map_err(|e| format!("couldn't start drawing: {e}"))
}

pub fn init_winit(event_loop: &EventLoop<'static, State>, state: &mut State) -> Result<(), String> {
    let (mut backend, source) = winit::init::<GlesRenderer>().map_err(|e| format!("couldn't open a window: {e}"))?;
    backend.window().set_title("Softcore");
    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = new_output("winit", "Window");
    let _global = output.create_global::<State>(&state.dh);
    // The window's picture is upside down to the renderer.
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, None);
    output.set_preferred(mode);
    renderer_ready(state, backend.renderer());
    let tracker = OutputDamageTracker::from_output(&output);
    state.add_output(output.clone());
    state.backend = Some(Backend::Winit(Winit { backend, unthrottled: false, output: output.clone(), tracker }));

    event_loop
        .handle()
        .insert_source(source, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.output_resized(&output);
            }
            WinitEvent::Input(event) => state.process_input(event, Some(&output)),
            WinitEvent::Redraw => {
                if let Some(mut backend) = state.backend.take() {
                    backend.draw(state);
                    state.backend = Some(backend);
                }
            }
            WinitEvent::CloseRequested => state.signal.stop(),
            WinitEvent::Focus(_) => {}
        })
        .map_err(|e| format!("couldn't watch the window: {e}"))?;
    start_frames(event_loop)
}

impl Winit {
    fn draw(&mut self, state: &mut State) {
        let size = self.backend.window_size();
        let shot = state.screenshot.take();
        let age = if shot.is_some() { 0 } else { self.backend.buffer_age().unwrap_or(0) };
        let egl_display = self.backend.renderer().egl_context().display().get_display_handle().handle;
        let first = !std::mem::replace(&mut self.unthrottled, true);
        let damage = {
            let (renderer, mut fb) = match self.backend.bind() {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("couldn't draw into the window: {e}");
                    return;
                }
            };
            let (elements, clear) = render::elements(state, renderer, &self.output);
            let damage = match self.tracker.render_output(renderer, &mut fb, age, &elements, clear) {
                Ok(r) => r.damage.cloned(),
                Err(e) => {
                    tracing::warn!("couldn't draw a frame: {e:?}");
                    return;
                }
            };
            if first {
                // A hidden window is sent no frames by the desktop it is on, and a swap that
                // waits for one would stop Softcore and every client with it. Never wait.
                unsafe {
                    smithay::backend::egl::ffi::egl::SwapInterval(egl_display, 0);
                }
            }
            if let Some(path) = shot
                && let Err(e) = render::save_screenshot(renderer, &fb, size, true, &path)
            {
                tracing::warn!("{e}");
            }
            damage
        };
        if let Some(damage) = damage {
            let _ = self.backend.submit(Some(&damage));
        }
        render::frame_done(state, &self.output);
    }
}

pub fn init_headless(event_loop: &EventLoop<'static, State>, state: &mut State, size: (i32, i32)) -> Result<(), String> {
    // Any GPU's render node will do: nothing is shown, frames are drawn into memory.
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri").map_err(|e| format!("no GPU to draw with (/dev/dri: {e})"))?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("renderD"))).collect();
    nodes.sort();
    let mut last = String::from("no GPU to draw with (no render node in /dev/dri)");
    for node in nodes {
        let made = (|| -> Result<GlesRenderer, String> {
            let file = std::fs::OpenOptions::new().read(true).write(true).open(&node).map_err(|e| e.to_string())?;
            let gbm = GbmDevice::new(file).map_err(|e| e.to_string())?;
            let egl = unsafe { EGLDisplay::new(gbm) }.map_err(|e| e.to_string())?;
            let ctx = EGLContext::new(&egl).map_err(|e| e.to_string())?;
            unsafe { GlesRenderer::new(ctx) }.map_err(|e| e.to_string())
        })();
        let mut renderer = match made {
            Ok(r) => r,
            Err(e) => {
                last = format!("couldn't draw with {}: {e}", node.display());
                continue;
            }
        };
        let target: GlesRenderbuffer = renderer.create_buffer(Fourcc::Abgr8888, (size.0, size.1).into()).map_err(|e| format!("couldn't make a frame to draw into: {e}"))?;
        let mode = Mode { size: size.into(), refresh: 60_000 };
        let output = new_output("headless", "Memory");
        let _global = output.create_global::<State>(&state.dh);
        output.change_current_state(Some(mode), Some(Transform::Normal), None, None);
        output.set_preferred(mode);
        renderer_ready(state, &mut renderer);
        let tracker = OutputDamageTracker::from_output(&output);
        state.add_output(output.clone());
        state.backend = Some(Backend::Headless(Headless { renderer, target, output, tracker }));
        return start_frames(event_loop);
    }
    Err(last)
}

impl Headless {
    fn draw(&mut self, state: &mut State) {
        let shot = state.screenshot.take();
        let age = if shot.is_some() { 0 } else { 1 };
        let (elements, clear) = render::elements(state, &mut self.renderer, &self.output);
        let size = self.output.current_mode().map(|m| m.size).unwrap_or_default();
        {
            let Ok(mut fb) = self.renderer.bind(&mut self.target) else { return };
            if let Err(e) = self.tracker.render_output(&mut self.renderer, &mut fb, age, &elements, clear) {
                tracing::warn!("couldn't draw a frame: {e:?}");
                return;
            }
            if let Some(path) = shot
                && let Err(e) = render::save_screenshot(&mut self.renderer, &fb, size, false, &path)
            {
                tracing::warn!("{e}");
            }
        }
        render::frame_done(state, &self.output);
    }
}
