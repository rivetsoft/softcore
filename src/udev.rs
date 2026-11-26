//! Softcore on displays of its own: started from a TTY (or a login manager), it takes the
//! seat, the GPU and the input devices itself. One GPU draws and shows everything.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{UdevBackend, UdevEvent, primary_gpu};
use smithay::output::{Mode, Output, PhysicalProperties};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{ModeTypeFlags, connector, crtc};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;
use smithay_drm_extras::display_info;
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};

use crate::backend::Backend;
use crate::render::{self, Elem};
use crate::state::State;

type Manager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type Surface = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

struct Screen {
    output: Output,
    global: smithay::reexports::wayland_server::backend::GlobalId,
    surface: Surface,
    /// A frame is on its way to the display; the next waits for it to be shown.
    pending: bool,
}

pub struct Udev {
    session: LibSeatSession,
    pub renderer: GlesRenderer,
    manager: Manager,
    scanner: DrmScanner,
    screens: HashMap<crtc::Handle, Screen>,
    /// Every display plugged in, in use or not, to set again when the settings change.
    known: Vec<(connector::Info, crtc::Handle)>,
    active: bool,
}

/// Run something with the display backend taken out of the state, so both can be changed.
fn with_udev(state: &mut State, f: impl FnOnce(&mut Udev, &mut State)) {
    if let Some(mut backend) = state.backend.take() {
        if let Backend::Udev(u) = &mut backend {
            f(u, state);
        }
        state.backend = Some(backend);
    }
}

pub fn init(event_loop: &EventLoop<'static, State>, state: &mut State) -> Result<(), String> {
    let (mut session, notifier) = LibSeatSession::new().map_err(|e| format!("couldn't take the seat ({e}). Softcore needs a TTY of its own, with seatd or logind running."))?;
    let seat = session.seat();

    let gpu = primary_gpu(&seat).ok().flatten().ok_or("no GPU with a display was found")?;
    let node = DrmNode::from_path(&gpu).map_err(|e| format!("couldn't use {}: {e}", gpu.display()))?;
    let fd = session.open(&gpu, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK).map_err(|e| format!("couldn't open {}: {e}", gpu.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_events) = DrmDevice::new(fd.clone(), true).map_err(|e| format!("couldn't use {}: {e}", gpu.display()))?;
    let gbm = GbmDevice::new(fd).map_err(|e| format!("couldn't use {}: {e}", gpu.display()))?;
    let egl = unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| format!("couldn't draw with {}: {e}", gpu.display()))?;
    let ctx = EGLContext::new(&egl).map_err(|e| format!("couldn't draw with {}: {e}", gpu.display()))?;
    let mut renderer = unsafe { GlesRenderer::new(ctx) }.map_err(|e| format!("couldn't draw with {}: {e}", gpu.display()))?;

    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let render_node = node.node_with_type(NodeType::Render).and_then(Result::ok);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node.into());
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let manager: Manager = DrmOutputManager::new(drm, allocator, exporter, Some(gbm), [Fourcc::Abgr8888, Fourcc::Argb8888], render_formats);

    crate::backend::renderer_ready(state, &mut renderer);
    state.backend = Some(Backend::Udev(Box::new(Udev { session: session.clone(), renderer, manager, scanner: DrmScanner::new(), screens: HashMap::new(), known: Vec::new(), active: true })));

    let handle = event_loop.handle();
    handle
        .insert_source(drm_events, |event, _, state| match event {
            DrmEvent::VBlank(crtc) => {
                with_udev(state, |u, _| {
                    if let Some(s) = u.screens.get_mut(&crtc) {
                        let _ = s.surface.frame_submitted();
                        s.pending = false;
                    }
                });
                // Straight on to the next frame when there is one to draw: slides run at the
                // display's own rate.
                crate::backend::draw_frame(state);
            }
            DrmEvent::Error(e) => tracing::warn!("the display: {e}"),
        })
        .map_err(|e| format!("couldn't watch the display: {e}"))?;

    // Keyboards, mice and touchpads.
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "couldn't find the input devices".to_string())?;
    handle
        .insert_source(LibinputInputBackend::new(libinput.clone()), |mut event, _, state| {
            if let InputEvent::DeviceAdded { device } = &mut event {
                let _ = device.config_tap_set_enabled(state.config.tap_to_click);
                let _ = device.config_scroll_set_natural_scroll_enabled(state.config.natural_scroll);
            }
            state.process_input(event, None);
        })
        .map_err(|e| format!("couldn't watch the input devices: {e}"))?;

    // Another TTY takes the screen, and gives it back.
    handle
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                libinput.suspend();
                with_udev(state, |u, _| {
                    u.active = false;
                    u.manager.pause();
                });
            }
            SessionEvent::ActivateSession => {
                let _ = libinput.resume();
                with_udev(state, |u, _| {
                    u.active = true;
                    if let Err(e) = u.manager.lock().activate(false) {
                        tracing::warn!("couldn't take the display back: {e}");
                    }
                    for s in u.screens.values_mut() {
                        s.pending = false;
                    }
                });
                state.queue_redraw();
            }
        })
        .map_err(|e| format!("couldn't watch the session: {e}"))?;

    // Displays plugged in and pulled out.
    let udev = UdevBackend::new(&seat).map_err(|e| format!("couldn't watch for displays: {e}"))?;
    handle
        .insert_source(udev, move |event, _, state| {
            if let UdevEvent::Changed { device_id } = event
                && device_id == node.dev_id()
            {
                with_udev(state, |u, state| u.scan(state));
            }
        })
        .map_err(|e| format!("couldn't watch for displays: {e}"))?;

    with_udev(state, |u, state| u.scan(state));
    if state.outputs.is_empty() {
        return Err("no display is connected".into());
    }
    crate::backend::start_frames(event_loop)
}

impl Udev {
    /// Whether the next frame waits: for the TTY to be Softcore's again, or for every
    /// display to show the frame it has (its vblank draws the next).
    pub fn waiting(&self) -> bool {
        !self.active || (!self.screens.is_empty() && self.screens.values().all(|s| s.pending))
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            tracing::warn!("couldn't switch to TTY {vt}: {e}");
        }
    }

    /// See which displays are there now.
    fn scan(&mut self, state: &mut State) {
        let Ok(found) = self.scanner.scan_connectors(self.manager.device()) else { return };
        for event in found {
            match event {
                DrmScanEvent::Connected { connector, crtc: Some(crtc) } => {
                    self.known.retain(|(c, _)| c.handle() != connector.handle());
                    self.known.push((connector.clone(), crtc));
                    self.connected(state, connector, crtc);
                }
                DrmScanEvent::Disconnected { connector, crtc: Some(crtc) } => {
                    self.known.retain(|(c, _)| c.handle() != connector.handle());
                    let name = Self::name(&connector);
                    state.off_outputs.retain(|o| o.name != name);
                    self.gone(state, crtc);
                }
                _ => {}
            }
        }
    }

    fn name(connector: &connector::Info) -> String {
        format!("{}-{}", connector.interface().as_str(), connector.interface_id())
    }

    /// The mode the settings ask of a display, else the display's own choice.
    fn pick(state: &State, connector: &connector::Info) -> Option<smithay::reexports::drm::control::Mode> {
        let name = Self::name(connector);
        let rule = state.config.outputs.iter().find(|r| r.name == name);
        let modes = connector.modes();
        let wanted = rule.and_then(|r| r.mode).and_then(|(w, h, hz)| {
            modes.iter().filter(|m| m.size() == (w as u16, h as u16)).min_by_key(|m| match hz {
                Some(hz) => (Mode::from(**m).refresh - hz).abs(),
                None => -Mode::from(**m).refresh,
            })
        });
        wanted.or(modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))).or(modes.first()).copied()
    }

    /// A display stops being used: unplugged, or turned off in the settings.
    fn gone(&mut self, state: &mut State, crtc: crtc::Handle) {
        if let Some(s) = self.screens.remove(&crtc) {
            state.dh.remove_global::<State>(s.global);
            state.remove_output(&s.output);
        }
    }

    /// The settings changed: displays turned off or on, and resolutions. A display whose
    /// resolution changes is taken down and set up again; its workspaces go next door for
    /// the moment and come straight home.
    pub fn reconfigure(&mut self, state: &mut State) {
        for (connector, crtc) in self.known.clone() {
            let name = Self::name(&connector);
            let off = state.config.outputs.iter().any(|r| r.name == name && r.off);
            let current = self.screens.get(&crtc).and_then(|s| s.output.current_mode());
            let same = current.is_some() && current == Self::pick(state, &connector).map(Mode::from);
            let on = current.is_some();
            if off {
                if on {
                    self.gone(state, crtc);
                }
                if !state.off_outputs.iter().any(|o| o.name == name) {
                    // Noted as there and not in use.
                    self.connected(state, connector, crtc);
                }
            } else if !on {
                self.connected(state, connector, crtc);
            } else if !same {
                self.gone(state, crtc);
                self.connected(state, connector, crtc);
            }
        }
        state.queue_redraw();
    }

    fn connected(&mut self, state: &mut State, connector: connector::Info, crtc: crtc::Handle) {
        let name = Self::name(&connector);
        let modes = connector.modes();
        let Some(drm_mode) = Self::pick(state, &connector) else { return };
        let mode = Mode::from(drm_mode);
        state.off_outputs.retain(|o| o.name != name);
        if state.config.outputs.iter().any(|r| r.name == name && r.off) {
            // Not used, but still told about, so it can be turned on again.
            let info = display_info::for_connector(self.manager.device(), connector.handle());
            state.off_outputs.push(crate::state::OffOutput {
                name,
                make: info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Unknown".into()),
                model: info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| "Unknown".into()),
                modes: modes.iter().map(|m| (m.size().0 as i32, m.size().1 as i32, Mode::from(*m).refresh, *m == drm_mode)).collect(),
            });
            return;
        }

        let device = self.manager.device();
        let info = display_info::for_connector(device, connector.handle());
        let (w, h) = connector.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (w as i32, h as i32).into(),
                subpixel: connector.subpixel().into(),
                make: info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Unknown".into()),
                model: info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| "Unknown".into()),
                serial_number: info.as_ref().and_then(|i| i.serial()).unwrap_or_else(|| "Unknown".into()),
            },
        );
        for m in modes {
            output.add_mode(Mode::from(*m));
        }
        output.set_preferred(mode);
        output.change_current_state(Some(mode), None, None, None);
        let planes = device.planes(&crtc).ok();
        let surface = match self.manager.lock().initialize_output::<_, Elem>(crtc, drm_mode, &[connector.handle()], &output, planes, &mut self.renderer, &DrmOutputRenderElements::default()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("couldn't use the display {name}: {e}");
                return;
            }
        };
        let global = output.create_global::<State>(&state.dh);
        state.add_output(output.clone());
        self.screens.insert(crtc, Screen { output, global, surface, pending: false });
    }

    pub fn draw(&mut self, state: &mut State) {
        if !self.active {
            return;
        }
        for screen in self.screens.values_mut() {
            if screen.pending {
                // Its turn comes when the frame before is shown.
                state.redraw = true;
                continue;
            }
            let (elements, clear) = render::elements(state, &mut self.renderer, &screen.output, true, true);
            match screen.surface.render_frame(&mut self.renderer, &elements, clear, FrameFlags::DEFAULT) {
                Ok(frame) => {
                    if !frame.is_empty {
                        match screen.surface.queue_frame(()) {
                            Ok(()) => screen.pending = true,
                            Err(e) => tracing::warn!("couldn't show a frame on {}: {e}", screen.output.name()),
                        }
                    }
                }
                Err(e) => tracing::warn!("couldn't draw a frame for {}: {e}", screen.output.name()),
            }
            render::frame_done(state, &screen.output);
        }
        crate::capture::after_draw(state, &mut self.renderer);
    }
}
