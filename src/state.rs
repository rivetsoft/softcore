//! Everything Softcore knows while it runs: the Wayland globals, the windows and where the
//! layout puts them, the displays, who has the keyboard.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::desktop::space::SpaceElement;
use smithay::desktop::{LayerSurface, PopupManager, Window, WindowSurfaceType, layer_map_for_output};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Size};
use smithay::wayland::compositor::{BufferAssignment, SurfaceAttributes, add_destruction_hook, add_pre_commit_hook, CompositorClientState, CompositorState, send_surface_state, with_states, with_surface_tree_downward, TraversalAction};
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState};
use smithay::wayland::fractional_scale::{FractionalScaleManagerState, with_fractional_scale};
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::idle_notify::IdleNotifierState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::session_lock::{LockSurface, SessionLockManagerState};
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer, WlrLayerShellState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::{SurfaceCachedState, XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::XdgActivationState;

use crate::config::{Action, Config, ModKey, WidthChange};
use crate::cursor::Cursors;
use crate::ipc::Ipc;
use crate::layout::{Layout, Rect, Tile, WinId};
use crate::render::Shaders;
use crate::ui::{Panel, Ui};

/// What each client carries.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// A display that is there and not in use.
pub struct OffOutput {
    pub name: String,
    pub make: String,
    pub model: String,
    /// Width, height, refresh in mHz, and whether it is the display's own choice.
    pub modes: Vec<(i32, i32, i32, bool)>,
}

/// Marks a surface whose commits are already watched for it leaving.
struct Watched;

/// A window on screen.
pub struct Win {
    pub window: Window,
    pub foreign: ForeignToplevelHandle,
    /// The display it was last told it is on.
    pub output: Option<String>,
    /// Told it has the whole screen while it keeps its place among the others.
    pub windowed: bool,
    pub deco: crate::render::Deco,
    /// Its piece of a strip of tabs, as last drawn: kept while the colour is the same.
    pub tab: Option<(smithay::backend::renderer::element::Id, [u8; 4])>,
    /// How far it has faded in since it appeared, from 0 to 1.
    pub shown: f64,
    /// Its last frame, taken as it leaves, to fade out with.
    pub picture: Option<smithay::backend::renderer::gles::GlesTexture>,
    /// How it looked before it began changing size, shown where its new frame doesn't
    /// reach yet.
    pub before: Option<(smithay::backend::renderer::gles::GlesTexture, smithay::backend::renderer::element::Id)>,
    /// When it last had the focus, for `softcore msg windows`.
    pub focused_at: Duration,
    /// What the settings' lines for windows like it give it.
    pub rule: crate::config::WindowRule,
}

#[derive(Default)]
pub struct Lock {
    pub locked: bool,
    pub surfaces: Vec<(Output, LockSurface)>,
}

pub struct State {
    pub start: Instant,
    pub dh: DisplayHandle,
    pub handle: LoopHandle<'static, State>,
    pub signal: LoopSignal,
    pub socket: String,
    /// In a window of another desktop (or with no screen at all), not on a display of its own.
    pub nested: bool,
    /// The X11 display kept for xwayland-satellite, if there is one.
    pub x11: Option<crate::xwayland::X11>,
    pub workspaces: crate::workspaces::Workspaces,
    /// A named scratchpad that was asked for while empty: its program has been started, and
    /// its window is shown as soon as it opens.
    pub scratch_wanted: Option<String>,
    /// The keys of a chord pressed so far, and which of what can follow is picked out.
    pub chord: Option<crate::chord::Chord>,

    pub config: Config,
    pub layout: Layout,
    pub wins: HashMap<WinId, Win>,
    /// Windows that haven't shown anything yet.
    pub unmapped: Vec<Window>,
    next_win: WinId,
    pub outputs: Vec<Output>,
    pub popups: PopupManager,

    pub seat: Seat<State>,
    /// In the space all displays share.
    pub pointer_pos: Point<f64, Logical>,
    pub cursor_status: CursorImageStatus,
    pub cursors: Cursors,
    /// Keys held down that started a binding: their release isn't the client's either.
    pub suppressed_keys: Vec<u32>,
    /// The keyboard layout in use is a virtual keyboard's, not the settings'.
    pub borrowed_keymap: bool,
    /// A panel or launcher that took the keyboard when clicked or opened.
    pub layer_focus: Option<LayerSurface>,
    pub lock: Lock,
    pub monitors_off: bool,

    pub backend: Option<crate::backend::Backend>,
    pub ui: Ui,
    pub shaders: Option<Shaders>,
    pub redraw: bool,
    /// Where the next frame drawn should also be saved.
    pub screenshot: Option<PathBuf>,
    pub ipc: Ipc,
    pub captures: crate::capture::Captures,
    /// Windows that have closed and are fading out.
    pub closing: Vec<crate::render::Closing>,
    pub watch: Option<crate::watch::Watch>,
    pub blur: crate::blur::Blur,
    /// A floating window being moved or resized with the pointer.
    pub drag: Option<crate::float::Drag>,
    pub overview: crate::overview::Overview,
    /// Displays that are plugged in and turned off in the settings: still listed, so they
    /// can be turned on again.
    pub off_outputs: Vec<OffOutput>,
    /// The window that gets every key while it has the focus, Softcore's own bindings
    /// included (all but the one that takes them back): a compositor being tried out in a
    /// window, a remote desktop, a virtual machine.
    pub all_keys: Option<WinId>,
    pub casts: crate::cast::Casts,
    /// The question of what to share, while it is being asked.
    pub picker: Option<crate::picker::Picker>,
    pub output_capture_source_state: smithay::wayland::image_capture_source::OutputCaptureSourceState,
    pub toplevel_capture_source_state: smithay::wayland::image_capture_source::ToplevelCaptureSourceState,
    pub image_copy_capture_state: smithay::wayland::image_copy_capture::ImageCopyCaptureState,
    last_frame: Instant,
    pub animating: bool,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    /// Held so the global stays up; nothing reads it.
    pub _xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub _output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub session_lock_state: SessionLockManagerState,
    pub foreign_toplevel_state: ForeignToplevelListState,
    pub idle_notifier_state: IdleNotifierState<State>,
    pub idle_inhibitors: Vec<WlSurface>,
    pub xdg_activation_state: XdgActivationState,
}

impl State {
    pub fn new(display: Display<State>, handle: LoopHandle<'static, State>, signal: LoopSignal, nested: bool, config: Config) -> State {
        let dh = display.handle();

        let source = ListeningSocketSource::new_auto().expect("couldn't open a Wayland socket");
        let socket = source.socket_name().to_string_lossy().into_owned();
        handle
            .insert_source(source, |stream, _, state: &mut State| {
                if let Err(e) = state.dh.insert_client(stream, Arc::new(ClientState::default())) {
                    tracing::warn!("couldn't add a client: {e}");
                }
            })
            .expect("couldn't listen on the Wayland socket");
        handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // Safety: the display is never dropped while the loop runs.
                unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                }
                Ok(PostAction::Continue)
            })
            .expect("couldn't watch the Wayland display");

        let compositor_state = CompositorState::new::<State>(&dh);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<State>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<State>(&dh);
        let data_control_state = DataControlState::new::<State, _>(&dh, Some(&primary_selection_state), |_| true);
        let session_lock_state = SessionLockManagerState::new::<State, _>(&dh, |_| true);
        let foreign_toplevel_state = ForeignToplevelListState::new::<State>(&dh);
        let idle_notifier_state = IdleNotifierState::new(&dh, handle.clone());
        let xdg_activation_state = XdgActivationState::new::<State>(&dh);
        ViewporterState::new::<State>(&dh);
        FractionalScaleManagerState::new::<State>(&dh);
        SinglePixelBufferState::new::<State>(&dh);
        CursorShapeManagerState::new::<State>(&dh);
        IdleInhibitManagerState::new::<State>(&dh);
        crate::workspaces::init(&dh);
        // Surfaces may say which part of themselves wants blur behind it.
        smithay::wayland::background_effect::BackgroundEffectState::new::<State>(&dh);
        // Typing and pointing by programs: on-screen keyboards, remote control, checks.
        smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<State, _>(&dh, |_| true);
        smithay::wayland::virtual_pointer::VirtualPointerManagerState::new::<State, _>(&dh, |_| true);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        let xkb = XkbConfig { layout: &config.xkb_layout, variant: &config.xkb_variant, options: (!config.xkb_options.is_empty()).then(|| config.xkb_options.clone()), ..Default::default() };
        if seat.add_keyboard(xkb, config.repeat_delay, config.repeat_rate).is_err() {
            seat.add_keyboard(XkbConfig::default(), config.repeat_delay, config.repeat_rate).expect("no keyboard layout could be loaded");
        }
        seat.add_pointer();

        let layout = Layout::new(config.layout.clone());
        let cursors = Cursors::new(&config.cursor_theme, config.cursor_size);
        let ipc = Ipc::start(&handle, &socket);

        let dh2 = dh.clone();
        State {
            start: Instant::now(),
            dh,
            handle,
            signal,
            socket,
            nested,
            x11: None,
            workspaces: Default::default(),
            scratch_wanted: None,
            chord: None,
            layout,
            wins: HashMap::new(),
            unmapped: Vec::new(),
            next_win: 1,
            outputs: Vec::new(),
            popups: PopupManager::default(),
            seat,
            pointer_pos: (0.0, 0.0).into(),
            cursor_status: CursorImageStatus::default_named(),
            cursors,
            suppressed_keys: Vec::new(),
            borrowed_keymap: false,
            layer_focus: None,
            lock: Lock::default(),
            monitors_off: false,
            backend: None,
            ui: Ui::new(),
            shaders: None,
            redraw: true,
            screenshot: None,
            ipc,
            captures: Default::default(),
            closing: Vec::new(),
            watch: None,
            blur: Default::default(),
            drag: None,
            overview: Default::default(),
            off_outputs: Vec::new(),
            all_keys: None,
            casts: Default::default(),
            picker: None,
            output_capture_source_state: smithay::wayland::image_capture_source::OutputCaptureSourceState::new::<State>(&dh2),
            toplevel_capture_source_state: smithay::wayland::image_capture_source::ToplevelCaptureSourceState::new::<State>(&dh2),
            image_copy_capture_state: smithay::wayland::image_copy_capture::ImageCopyCaptureState::new::<State>(&dh2),
            last_frame: Instant::now(),
            animating: false,
            config,
            compositor_state,
            xdg_shell_state,
            _xdg_decoration_state: xdg_decoration_state,
            layer_shell_state,
            shm_state,
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            _output_manager_state: output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            session_lock_state,
            foreign_toplevel_state,
            idle_notifier_state,
            idle_inhibitors: Vec::new(),
            xdg_activation_state,
        }
    }

    pub fn queue_redraw(&mut self) {
        self.redraw = true;
    }

    /// Whether Mod is Alt (in a window of another desktop) or Super.
    pub fn mod_is_alt(&self) -> bool {
        match self.config.mod_key {
            ModKey::Alt => true,
            ModKey::Super => false,
            ModKey::Auto => self.nested,
        }
    }

    // ---- displays ----

    pub fn output_size(output: &Output) -> Size<i32, Logical> {
        let Some(mode) = output.current_mode() else { return (0, 0).into() };
        let size = output.current_transform().transform_size(mode.size);
        size.to_f64().to_logical(output.current_scale().fractional_scale()).to_i32_round()
    }

    pub fn output_geometry(output: &Output) -> Rectangle<i32, Logical> {
        Rectangle::new(output.current_location(), Self::output_size(output))
    }

    /// The way the settings turn a display. Not a window on another desktop: its picture
    /// has a way up of its own.
    fn rule_transform(output: &Output, rule: Option<&crate::config::OutputRule>) -> Option<smithay::utils::Transform> {
        use smithay::utils::Transform as T;
        let (turn, flip) = rule?.transform.filter(|_| output.name() != "winit")?;
        Some(match (flip, turn) {
            (false, 90) => T::_90,
            (false, 180) => T::_180,
            (false, 270) => T::_270,
            (true, 90) => T::Flipped90,
            (true, 180) => T::Flipped180,
            (true, 270) => T::Flipped270,
            (true, _) => T::Flipped,
            (false, _) => T::Normal,
        })
    }

    pub fn add_output(&mut self, output: Output) {
        let rule = self.config.outputs.iter().find(|r| r.name == output.name()).cloned();
        let x = self.outputs.iter().map(|o| Self::output_geometry(o)).map(|g| g.loc.x + g.size.w).max().unwrap_or(0);
        let loc = rule.as_ref().and_then(|r| r.position).unwrap_or((x, 0));
        let scale = rule.as_ref().and_then(|r| r.scale).map(smithay::output::Scale::Fractional);
        output.change_current_state(None, Self::rule_transform(&output, rule.as_ref()), scale, Some(loc.into()));
        let size = Self::output_size(&output);
        self.layout.add_monitor(&output.name(), (size.w, size.h));
        self.layout.set_scroll(&output.name(), rule.as_ref().and_then(|r| r.scroll));
        self.apply_names();
        if self.outputs.is_empty() {
            self.pointer_pos = (loc.0 as f64 + size.w as f64 / 2.0, loc.1 as f64 + size.h as f64 / 2.0).into();
        }
        self.outputs.push(output);
        self.update_areas();
        self.queue_redraw();
    }

    pub fn remove_output(&mut self, output: &Output) {
        self.outputs.retain(|o| o != output);
        self.layout.remove_monitor(&output.name());
        self.lock.surfaces.retain(|(o, _)| o != output);
        self.queue_redraw();
    }

    /// A display changed size (or scale): the layout and the panels follow.
    pub fn output_resized(&mut self, output: &Output) {
        layer_map_for_output(output).arrange();
        for (o, s) in &self.lock.surfaces {
            if o == output {
                let size = Self::output_size(output);
                s.with_pending_state(|st| st.size = Some((size.w as u32, size.h as u32).into()));
                s.send_configure();
            }
        }
        self.update_areas();
        self.queue_redraw();
    }

    /// Give the layout the workspaces and scratchpads the settings name.
    pub fn apply_names(&mut self) {
        let wanted: Vec<_> = self.config.workspaces.iter().map(|w| (w.name.clone(), w.output.clone(), w.scroll)).collect();
        self.layout.set_named_workspaces(&wanted);
        self.layout.set_scratchpad_sizes(self.config.scratchpads.iter().filter_map(|s| s.size.map(|z| (s.name.clone(), z))).collect());
    }

    pub fn output_named(&self, name: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.name() == name)
    }

    /// The display with the focus.
    pub fn active_output(&self) -> Option<&Output> {
        self.layout.active_monitor().and_then(|m| self.output_named(&m.name)).or(self.outputs.first())
    }

    pub fn output_at(&self, pos: Point<f64, Logical>) -> Option<&Output> {
        self.outputs.iter().find(|o| Self::output_geometry(o).to_f64().contains(pos))
    }

    /// Tell the layout what room the panels leave on each display.
    pub fn update_areas(&mut self) {
        for o in &self.outputs {
            let size = Self::output_size(o);
            let zone = layer_map_for_output(o).non_exclusive_zone();
            if self.layout.set_geometry(&o.name(), (size.w, size.h), Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h)) {
                self.redraw = true;
            }
        }
    }

    // ---- windows ----

    pub fn win_for_surface(&self, surface: &WlSurface) -> Option<WinId> {
        self.wins.iter().find(|(_, w)| w.window.toplevel().is_some_and(|t| t.wl_surface() == surface)).map(|(id, _)| *id)
    }

    /// The size a floating window has chosen.
    pub fn float_size(&self) -> impl Fn(WinId) -> (i32, i32) + '_ {
        |id| self.wins.get(&id).map(|w| w.window.geometry().size).map(|s| (s.w.max(1), s.h.max(1))).unwrap_or((1, 1))
    }

    /// Dialogs and windows that can't be resized float over the columns.
    pub fn wants_floating(window: &Window) -> bool {
        let Some(top) = window.toplevel() else { return false };
        if top.parent().is_some() {
            return true;
        }
        with_states(top.wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let s = guard.current();
            s.min_size.w > 0 && s.min_size.h > 0 && s.min_size == s.max_size
        })
    }

    pub fn title_and_app_id(window: &Window) -> (String, String) {
        let Some(top) = window.toplevel() else { return Default::default() };
        with_states(top.wl_surface(), |states| {
            let data = states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
            (data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default())
        })
    }

    /// A window has drawn its first frame: into the layout it goes.
    pub fn map_window(&mut self, window: Window) {
        let id = self.next_win;
        self.next_win += 1;
        let (title, app_id) = Self::title_and_app_id(&window);
        let foreign = self.foreign_toplevel_state.new_toplevel::<State>(&title, &app_id);
        let rule = self.config.window_rule(&app_id, &title);
        let floating = rule.floating.unwrap_or_else(|| Self::wants_floating(&window));
        if let Some(surface) = window.toplevel().map(|t| t.wl_surface().clone())
            && with_states(&surface, |s| s.data_map.insert_if_missing(|| Watched))
        {
            // Its picture is needed after it has gone, to fade out with, and has to be taken
            // while there still is one: before a commit that takes the frame away, and
            // before the surface goes.
            add_pre_commit_hook::<State, _>(&surface, |state, _, surface| {
                let leaving = with_states(surface, |s| matches!(s.cached_state.get::<SurfaceAttributes>().pending().buffer, Some(BufferAssignment::Removed)));
                if leaving && let Some(id) = state.win_for_surface(surface) {
                    crate::render::remember(state, id);
                }
            });
            add_destruction_hook::<State, _>(&surface, |state, surface| {
                if let Some(id) = state.win_for_surface(surface) {
                    crate::render::remember(state, id);
                }
            });
        }
        self.wins.insert(id, Win { window, foreign, output: None, windowed: false, deco: Default::default(), tab: None, shown: if self.config.animations { 0.0 } else { 1.0 }, picture: None, before: None, focused_at: self.start.elapsed(), rule: rule.clone() });
        if let Some(name) = &rule.scratchpad {
            // Put away as it opens; shown at once when it is what the scratchpad key is
            // waiting for.
            self.layout.put_away(id, Some(name));
            if self.scratch_wanted.as_deref() == Some(name.as_str()) {
                self.scratch_wanted = None;
                self.layout.toggle_scratchpad(Some(name));
            }
        } else {
            let open = crate::layout::Open { floating, output: rule.output.as_deref(), workspace: rule.workspace.as_ref(), width: rule.width, maximized: rule.maximized == Some(true), focus: rule.focus != Some(false) };
            self.layout.open_window(id, open);
            if rule.fullscreen == Some(true) {
                self.layout.set_fullscreen(id, Some(true));
            }
        }
        if rule.focus != Some(false) {
            self.layer_focus = None;
        }
        self.queue_redraw();
    }

    pub fn unmap_window(&mut self, id: WinId) -> Option<Window> {
        crate::render::start_closing(self, id);
        if self.all_keys == Some(id) {
            self.all_keys = None;
        }
        let win = self.wins.remove(&id)?;
        self.layout.remove_window(id);
        self.foreign_toplevel_state.remove_toplevel(&win.foreign);
        for o in &self.outputs {
            win.window.output_leave(o);
        }
        self.queue_redraw();
        Some(win.window)
    }

    /// The windows on a display now, bottom first.
    pub fn tiles(&self, output: &Output) -> Vec<Tile> {
        match self.layout.monitor(&output.name()) {
            Some(m) => match self.overview_map(&self.layout.mons[m]) {
                Some(map) => self.overview_tiles(m, &map),
                None => self.layout.tiles(m, &self.float_size()),
            },
            None => Vec::new(),
        }
    }

    /// Tell every window its size and whether it is the active one, and which display it is on.
    fn sync_windows(&mut self) {
        let active = self.layout.active_window();
        for mi in 0..self.layout.mons.len() {
            let name = self.layout.mons[mi].name.clone();
            let Some(output) = self.output_named(&name).cloned() else { continue };
            let sizes = self.layout.sizes(mi, &self.float_size());
            for t in sizes {
                // A floating window chooses its size, but one from the scratchpad is given one.
                let scratch = self.layout.float_size(t.id);
                let Some(win) = self.wins.get_mut(&t.id) else { continue };
                let Some(top) = win.window.toplevel() else { continue };
                top.with_pending_state(|s| {
                    s.size = if t.floating { scratch.map(Into::into) } else { Some((t.rect.w, t.rect.h).into()) };
                    for st in [xdg_toplevel::State::TiledLeft, xdg_toplevel::State::TiledRight, xdg_toplevel::State::TiledTop, xdg_toplevel::State::TiledBottom] {
                        if t.floating {
                            s.states.unset(st);
                        } else {
                            s.states.set(st);
                        }
                    }
                    if t.fullscreen || win.windowed {
                        s.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        s.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                    if active == Some(t.id) {
                        s.states.set(xdg_toplevel::State::Activated);
                    } else {
                        s.states.unset(xdg_toplevel::State::Activated);
                    }
                });
                if top.is_initial_configure_sent() {
                    top.send_pending_configure();
                }
                if win.output.as_deref() != Some(name.as_str()) {
                    for o in &self.outputs {
                        if *o != output {
                            win.window.output_leave(o);
                        }
                    }
                    win.window.output_enter(&output, Rectangle::from_size((i32::MAX, i32::MAX).into()));
                    win.output = Some(name.clone());
                    send_scale(win.window.toplevel().unwrap().wl_surface(), &output);
                }
            }
        }
    }

    // ---- who has the keyboard ----

    fn focus_target(&self) -> Option<WlSurface> {
        if self.lock.locked {
            let on = self.active_output();
            return self.lock.surfaces.iter().find(|(o, _)| Some(o) == on).or(self.lock.surfaces.first()).map(|(_, s)| s.wl_surface().clone());
        }
        if self.ui.panel != Panel::None {
            return None;
        }
        // A launcher or menu that asks for every key gets them, wherever the pointer is.
        for o in &self.outputs {
            let map = layer_map_for_output(o);
            for layer in [Layer::Overlay, Layer::Top] {
                if let Some(l) = map.layers_on(layer).rev().find(|l| l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive) {
                    return Some(l.wl_surface().clone());
                }
            }
        }
        if let Some(l) = &self.layer_focus {
            return Some(l.wl_surface().clone());
        }
        let id = self.layout.active_window()?;
        self.wins.get(&id).and_then(|w| w.window.toplevel()).map(|t| t.wl_surface().clone())
    }

    pub fn update_focus(&mut self) {
        if self.layer_focus.as_ref().is_some_and(|l| !l.alive() || l.cached_state().keyboard_interactivity == KeyboardInteractivity::None) {
            self.layer_focus = None;
        }
        let target = self.focus_target();
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        // An open menu holds the keyboard until it closes.
        if keyboard.is_grabbed() {
            return;
        }
        if keyboard.current_focus() != target {
            keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
            if let Some(id) = self.layout.active_window() {
                let now = self.start.elapsed();
                if let Some(w) = self.wins.get_mut(&id) {
                    w.focused_at = now;
                }
            }
        }
    }

    /// What is under the pointer can change without the pointer moving (a window opens, a
    /// column scrolls away): tell the old surface it left and the new one it entered.
    fn update_pointer_focus(&mut self) {
        if let CursorImageStatus::Surface(s) = &self.cursor_status
            && !s.alive()
        {
            self.cursor_status = CursorImageStatus::default_named();
        }
        let Some(pointer) = self.seat.get_pointer() else { return };
        if pointer.is_grabbed() {
            return;
        }
        let under = self.surface_under(self.pointer_pos);
        if pointer.current_focus() != under.as_ref().map(|(s, _)| s.clone()) {
            let time = self.start.elapsed().as_millis() as u32;
            pointer.motion(self, under, &smithay::input::pointer::MotionEvent { location: self.pointer_pos, serial: SERIAL_COUNTER.next_serial(), time: smithay::backend::input::InputTime::from_millis(time) });
            pointer.frame(self);
        }
    }

    // ---- once round the loop ----

    /// After every batch of events: drop what has gone, and bring windows, focus and anyone
    /// listening up to date.
    pub fn refresh(&mut self) {
        let dead: Vec<WinId> = self.wins.iter().filter(|(_, w)| !w.window.alive()).map(|(id, _)| *id).collect();
        for id in dead {
            self.unmap_window(id);
        }
        self.unmapped.retain(|w| w.alive());
        self.lock.surfaces.retain(|(_, s)| s.alive());
        self.popups.cleanup();
        for o in &self.outputs {
            layer_map_for_output(o).cleanup();
        }
        self.foreign_toplevel_state.cleanup_closed_handles();
        self.update_areas();
        // Before windows are told their new sizes, while they still look as they did.
        crate::render::keep_before(self);
        self.sync_windows();
        self.update_focus();
        self.update_pointer_focus();
        self.update_capture_sizes();
        crate::cast::refresh(self);
        crate::picker::refresh(self);
        crate::ipc::notify(self);
        crate::workspaces::refresh(self);
        let _ = self.dh.flush_clients();
    }

    /// Move everything that is sliding or fading on; true when a frame should be drawn.
    pub fn advance(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.1);
        self.last_frame = now;
        let mut moving = self.layout.advance(dt);
        // Windows fading in, and out.
        let step = if self.config.animations { dt * self.config.animation_speed / 0.2 } else { 1.0 };
        for w in self.wins.values_mut().filter(|w| w.shown < 1.0) {
            w.shown = (w.shown + step).min(1.0);
            moving = true;
        }
        let to = if self.overview.on { 1.0 } else { 0.0 };
        if self.overview.p != to {
            self.overview.p = if self.overview.on { (self.overview.p + step).min(1.0) } else { (self.overview.p - step).max(0.0) };
            moving = true;
        }
        moving |= !self.closing.is_empty();
        self.closing.retain_mut(|c| {
            c.t += step;
            c.t < 1.0
        });
        let was = std::mem::replace(&mut self.animating, moving);
        let wanted = std::mem::take(&mut self.redraw);
        wanted || moving || was
    }

    /// The moment a watched file changes, and every two seconds besides: the settings and
    /// the desktop's colours.
    pub fn check_files(&mut self) {
        if self.config.changed_on_disk() {
            self.reload_config();
        }
        if self.ui.check_look() {
            self.queue_redraw();
        }
    }

    /// Back to the keyboard layout of the settings, after a virtual keyboard used its own.
    pub fn restore_keymap(&mut self) {
        if !std::mem::take(&mut self.borrowed_keymap) {
            return;
        }
        let Some(k) = self.seat.get_keyboard() else { return };
        let c = self.config.clone();
        let xkb = XkbConfig { layout: &c.xkb_layout, variant: &c.xkb_variant, options: (!c.xkb_options.is_empty()).then(|| c.xkb_options.clone()), ..Default::default() };
        if k.set_xkb_config(self, xkb).is_err() {
            let _ = k.set_xkb_config(self, XkbConfig::default());
        }
    }

    pub fn reload_config(&mut self) {
        let new = crate::config::load();
        let old = std::mem::replace(&mut self.config, new);
        self.layout.set_options(self.config.layout.clone());
        self.apply_names();
        self.chord = None;
        for w in self.wins.values_mut() {
            let (title, app_id) = Self::title_and_app_id(&w.window);
            w.rule = self.config.window_rule(&app_id, &title);
        }
        if let Some(k) = self.seat.get_keyboard() {
            if (&old.xkb_layout, &old.xkb_variant, &old.xkb_options) != (&self.config.xkb_layout, &self.config.xkb_variant, &self.config.xkb_options) {
                let c = self.config.clone();
                let xkb = XkbConfig { layout: &c.xkb_layout, variant: &c.xkb_variant, options: (!c.xkb_options.is_empty()).then(|| c.xkb_options.clone()), ..Default::default() };
                if k.set_xkb_config(self, xkb).is_err() {
                    self.config.errors.push("the keyboard layout couldn't be loaded".into());
                }
            }
            k.change_repeat_info(self.config.repeat_rate, self.config.repeat_delay);
        }
        if (&old.cursor_theme, old.cursor_size) != (&self.config.cursor_theme, self.config.cursor_size) {
            self.cursors = Cursors::new(&self.config.cursor_theme, self.config.cursor_size);
        }
        for o in self.outputs.clone() {
            self.layout.set_scroll(&o.name(), self.config.outputs.iter().find(|r| r.name == o.name()).and_then(|r| r.scroll));
            if let Some(r) = self.config.outputs.iter().find(|r| r.name == o.name()).cloned() {
                o.change_current_state(None, Self::rule_transform(&o, Some(&r)), r.scale.map(smithay::output::Scale::Fractional), r.position.map(Into::into));
                self.output_resized(&o);
            }
        }
        // Resolutions, and displays turned off and on: real displays only.
        if let Some(mut backend) = self.backend.take() {
            if let crate::backend::Backend::Udev(u) = &mut backend {
                u.reconfigure(self);
            }
            self.backend = Some(backend);
        }
        self.ui.settings_changed(&self.config);
        if !self.config.errors.is_empty() {
            self.ui.show(Panel::Errors);
        } else if self.ui.panel == Panel::Errors {
            self.ui.show(Panel::None);
        }
        self.queue_redraw();
    }

    // ---- actions ----

    pub fn spawn(&self, command: &str) {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(command);
        c.env("WAYLAND_DISPLAY", &self.socket).env("SOFTCORE_SOCKET", self.ipc.path()).env("XDG_CURRENT_DESKTOP", "softcore").env("XDG_SESSION_DESKTOP", "softcore").env("XDG_SESSION_TYPE", "wayland");
        // In a window of another desktop, its own sockets would send clients' questions there.
        for k in ["NIRI_SOCKET", "HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "DISPLAY"] {
            c.env_remove(k);
        }
        if let Some(x11) = &self.x11 {
            c.env("DISPLAY", format!(":{}", x11.display));
        }
        // What Softcore starts can ask it things with `softcore msg` (Rivet Shell does, for
        // its workspaces), wherever this Softcore was started from.
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
            let path = std::env::var_os("PATH").unwrap_or_default();
            if !std::env::split_paths(&path).any(|p| p == dir)
                && let Ok(joined) = std::env::join_paths(std::env::split_paths(&path).chain([dir]))
            {
                c.env("PATH", joined);
            }
        }
        for (k, v) in &self.config.env {
            c.env(k, v);
        }
        c.stdin(std::process::Stdio::null());
        // Softcore doesn't wait for what it starts (see main), and a program started that way
        // would inherit not being able to wait for anything either: every command it ran
        // itself would seem to fail. Give it back the usual way.
        unsafe {
            use std::os::unix::process::CommandExt;
            c.pre_exec(|| {
                libc::signal(libc::SIGCHLD, libc::SIG_DFL);
                Ok(())
            });
        }
        if let Err(e) = c.spawn() {
            tracing::warn!("couldn't run {command:?}: {e}");
        }
    }

    pub fn do_action(&mut self, action: Action) {
        if crate::float::action(self, &action) {
            self.queue_redraw();
            return;
        }
        use crate::layout::Dir;
        let active = self.layout.active_window();
        match action {
            Action::Spawn(c) => self.spawn(&c),
            Action::Quit { confirm: true } => self.ui.show(Panel::Quit),
            Action::Quit { confirm: false } => self.signal.stop(),
            Action::CloseWindow => {
                if let Some(top) = active.and_then(|id| self.wins.get(&id)).and_then(|w| w.window.toplevel()) {
                    top.send_close();
                }
            }
            Action::Focus(d) => self.layout.focus(d),
            Action::FocusColumnFirst => self.layout.focus_column_edge(false),
            Action::FocusColumnLast => self.layout.focus_column_edge(true),
            Action::MoveColumn(d) => self.layout.move_column(d),
            Action::ConsumeOrExpel(d) => self.layout.consume_or_expel(if d == Dir::Left { Dir::Left } else { Dir::Right }),
            Action::SwitchPresetWidth => self.layout.switch_preset_width(),
            Action::SetColumnWidth(WidthChange::By(b)) => self.layout.change_width(b),
            Action::SetColumnWidth(WidthChange::To(w)) => self.layout.set_width(w),
            Action::SetWindowHeight(WidthChange::By(b)) => self.layout.set_height(Some(b), true),
            Action::SetWindowHeight(WidthChange::To(h)) => self.layout.set_height(Some(h), false),
            Action::ResetWindowHeight => self.layout.set_height(None, false),
            Action::MaximizeColumn => self.layout.toggle_maximized(),
            Action::ToggleTabbed => self.layout.toggle_tabbed(),
            Action::WorkspaceScroll(to) => self.layout.set_workspace_scroll(to),
            Action::FullscreenWindow => {
                if let Some(id) = active {
                    if let Some(w) = self.wins.get_mut(&id) {
                        w.windowed = false;
                    }
                    self.layout.set_fullscreen(id, None);
                }
            }
            Action::WindowedFullscreen => {
                if let Some(id) = active {
                    self.layout.set_fullscreen(id, Some(false));
                    if let Some(w) = self.wins.get_mut(&id) {
                        w.windowed = !w.windowed;
                    }
                }
            }
            Action::CenterColumn => self.layout.center_column(),
            Action::ToggleFloating => {
                if let Some(id) = active {
                    self.layout.toggle_floating(id);
                }
            }
            // Put away, a window fades out as one that closed does; shown, it fades in.
            Action::MoveToScratchpad(name) => {
                if let Some(id) = active {
                    crate::render::start_closing(self, id);
                    self.layout.move_to_scratchpad(name.as_deref());
                }
            }
            Action::ToggleScratchpad(name) => {
                for id in self.layout.scratchpad_in_view(name.as_deref()) {
                    crate::render::start_closing(self, id);
                }
                match self.layout.toggle_scratchpad(name.as_deref()) {
                    crate::layout::Scratch::Shown(id) => {
                        if self.config.animations
                            && let Some(w) = self.wins.get_mut(&id)
                        {
                            w.shown = 0.0;
                        }
                    }
                    // A named scratchpad with nothing in it starts its program, and shows
                    // the window when it comes.
                    crate::layout::Scratch::Empty => {
                        let command = name.as_ref().and_then(|n| self.config.scratchpads.iter().find(|s| s.name == *n)).and_then(|s| s.command.clone());
                        if let Some(c) = command {
                            self.scratch_wanted = name;
                            self.spawn(&c);
                        }
                    }
                    crate::layout::Scratch::Hidden => {}
                }
            }
            Action::ToggleShortcutsInhibit => self.all_keys = if self.all_keys.is_some() && self.all_keys == active { None } else { active },
            Action::SwitchFloatingFocus => self.layout.switch_floating_focus(),
            Action::FocusWorkspace(w) => {
                let was = self.layout.active_mon;
                self.layout.focus_workspace_ref(&w);
                if self.layout.active_mon != was {
                    self.warp_pointer_to_active_output();
                }
            }
            Action::FocusWorkspaceBy(b) => self.layout.focus_workspace_by(b),
            Action::MoveToWorkspace { to, column } => self.layout.move_to_workspace_ref(&to, column),
            Action::MoveToWorkspaceBy { by, column } => self.layout.move_to_workspace_by(by, column),
            Action::FocusMonitor(b) => {
                if self.layout.focus_monitor(b) {
                    self.warp_pointer_to_active_output();
                }
            }
            Action::MoveColumnToMonitor(b) => self.layout.move_column_to_monitor(b),
            Action::Overview(on) => {
                self.set_overview(on.unwrap_or(!self.overview.on));
                return;
            }
            Action::ShowHotkeyOverlay => self.ui.show(if self.ui.panel == Panel::Keys { Panel::None } else { Panel::Keys }),
            Action::PowerOffMonitors => self.monitors_off = true,
            Action::PowerOnMonitors => self.monitors_off = false,
            Action::ReloadConfig => self.reload_config(),
            Action::ChangeVt(n) => {
                if let Some(b) = self.backend.as_mut() {
                    b.change_vt(n);
                }
            }
            Action::Screenshot(path) => {
                let path = path.map(PathBuf::from).unwrap_or_else(|| {
                    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    rivet_xdg::places::home().join(format!("Pictures/Screenshots/softcore-{secs}.png"))
                });
                self.screenshot = Some(path);
            }
        }
        self.layer_focus = None;
        self.queue_redraw();
    }

    fn warp_pointer_to_active_output(&mut self) {
        if let Some(g) = self.active_output().map(Self::output_geometry) {
            self.pointer_pos = (g.loc.x as f64 + g.size.w as f64 / 2.0, g.loc.y as f64 + g.size.h as f64 / 2.0).into();
        }
    }

    // ---- what is under the pointer ----

    /// The surface at a point, and where that surface's corner is.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        // An open panel of Softcore's own takes the pointer: nothing under it is clicked.
        if self.ui.panel != Panel::None && !self.lock.locked {
            return None;
        }
        let output = self.output_at(pos)?;
        let origin = Self::output_geometry(output).loc.to_f64();
        let local = pos - origin;
        if self.lock.locked {
            return self.lock.surfaces.iter().find(|(o, _)| o == output).map(|(_, s)| (s.wl_surface().clone(), origin));
        }
        let map = layer_map_for_output(output);
        let layer_under = |layer: Layer| {
            let l = map.layer_under(layer, local)?;
            let geo = map.layer_geometry(l)?;
            l.surface_under(local - geo.loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + geo.loc).to_f64() + origin))
        };
        // In the overview the windows are pictures to pick from: only the shell's panels,
        // which stay as they are, hear from the pointer.
        if self.overview.on {
            return layer_under(Layer::Overlay).or_else(|| layer_under(Layer::Top));
        }
        let tiles = self.tiles(output);
        let window_under = |fullscreen: bool| {
            tiles.iter().rev().filter(|t| t.fullscreen == fullscreen && !t.hidden).find_map(|t| {
                let w = &self.wins.get(&t.id)?.window;
                let loc = Point::<i32, Logical>::from((t.rect.x, t.rect.y)) - w.geometry().loc;
                w.surface_under(local - loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + loc).to_f64() + origin))
            })
        };
        layer_under(Layer::Overlay)
            .or_else(|| window_under(true))
            .or_else(|| layer_under(Layer::Top))
            .or_else(|| window_under(false))
            .or_else(|| layer_under(Layer::Bottom))
            .or_else(|| layer_under(Layer::Background))
    }

    /// The window at a point (its popups count as the window).
    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<WinId> {
        let (surface, _) = self.surface_under(pos)?;
        let root = root_surface(&surface);
        self.win_for_surface(&root).or_else(|| {
            // A popup: find the window it belongs to.
            let popup = self.popups.find_popup(&root)?;
            let parent = smithay::desktop::find_popup_root_surface(&popup).ok()?;
            self.win_for_surface(&parent)
        })
    }

    pub fn layer_under(&self, pos: Point<f64, Logical>) -> Option<LayerSurface> {
        let (surface, _) = self.surface_under(pos)?;
        let root = root_surface(&surface);
        let root = match self.popups.find_popup(&root) {
            Some(p) => smithay::desktop::find_popup_root_surface(&p).ok()?,
            None => root,
        };
        self.outputs.iter().find_map(|o| layer_map_for_output(o).layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).cloned())
    }
}

pub fn root_surface(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
        root = parent;
    }
    root
}

/// Tell a surface and everything under it the scale of the display it is on.
pub fn send_scale(surface: &WlSurface, output: &Output) {
    let scale = output.current_scale();
    let transform = output.current_transform();
    with_surface_tree_downward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |s, states, _| {
            send_surface_state(s, states, scale.integer_scale(), transform);
            with_fractional_scale(states, |f| f.set_preferred_scale(scale.fractional_scale()));
        },
        |_, _, _| true,
    );
}
