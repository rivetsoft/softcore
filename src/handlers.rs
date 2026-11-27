//! What Softcore does when clients ask for things: the Wayland protocols it speaks.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::{LayerSurface, PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, Window, WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output};
use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::{CursorImageStatus, Focus};
use smithay::input::tablet::TabletSeatHandler;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{Rectangle, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState, is_sync_subsurface, with_states};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState};
use smithay::wayland::fractional_scale::FractionalScaleHandler;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::pointer_constraints::PointerConstraintsHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus};
use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState, set_primary_focus};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker};
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData};

use crate::state::{ClientState, State, root_surface, send_scale};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.queue_redraw();
        if is_sync_subsurface(surface) {
            return;
        }
        let root = root_surface(surface);
        let has_buffer = |s: &WlSurface| with_renderer_surface_state(s, |st| st.buffer().is_some()).unwrap_or(false);

        // A window that hasn't shown anything yet: answer its first commit with the size it
        // will have, and take it into the layout when a frame arrives.
        if let Some(i) = self.unmapped.iter().position(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &root)) {
            self.unmapped[i].on_commit();
            if has_buffer(&root) {
                let window = self.unmapped.remove(i);
                self.map_window(window);
            } else if surface == &root {
                let window = &self.unmapped[i];
                let top = window.toplevel().unwrap();
                if !top.is_initial_configure_sent() {
                    let (title, app_id) = State::title_and_app_id(window);
                    let rule = self.config.window_rule(&app_id, &title);
                    let floating = rule.floating.unwrap_or_else(|| State::wants_floating(window) || self.layout.floats_new(rule.output.as_deref(), rule.workspace.as_ref())) || rule.scratchpad.is_some();
                    let bar = self.layout.bar_height(rule.title_bar, floating);
                    let size = (!floating).then(|| self.layout.new_window_size(rule.output.as_deref(), bar)).flatten();
                    top.with_pending_state(|s| s.size = size.map(Into::into));
                    top.send_configure();
                }
            }
            return;
        }

        if let Some(id) = self.win_for_surface(&root) {
            self.wins[&id].window.on_commit();
            if surface == &root && !has_buffer(&root) {
                // It took its frame away: back to waiting for one.
                if let Some(w) = self.unmap_window(id) {
                    self.unmapped.push(w);
                }
            }
            return;
        }

        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
        {
            let _ = popup.send_configure();
            return;
        }

        // A panel, wallpaper or launcher.
        for output in self.outputs.clone() {
            let mut map = layer_map_for_output(&output);
            let Some(layer) = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).cloned() else { continue };
            map.arrange();
            let sent = with_states(&root, |states| states.data_map.get::<LayerSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent));
            if !sent {
                layer.layer_surface().send_configure();
            }
            drop(map);
            // One that takes keys when asked: give them as it appears, as a launcher expects.
            let wants = layer.cached_state().keyboard_interactivity;
            if has_buffer(&root) && wants == KeyboardInteractivity::OnDemand && matches!(layer.layer(), Layer::Top | Layer::Overlay) && self.layer_focus.is_none() && !layer.user_data().get::<Shown>().is_some() {
                layer.user_data().insert_if_missing(|| Shown);
                self.layer_focus = Some(layer);
            }
            return;
        }
    }
}

/// Marks a layer surface that has been given the keyboard once on appearing.
struct Shown;

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        let ok = self.backend.as_mut().is_some_and(|b| b.renderer().import_dmabuf(&dmabuf, None).is_ok());
        if ok {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Softcore draws the border, and the title bar where the settings ask for one.
        surface.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        self.unmapped.push(Window::new_wayland_window(surface));
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        // A menu takes the keyboard and the pointer while it is open, and is told to close
        // when a click lands outside its app.
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else { return };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed() && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed() && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial()))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        if let Some(id) = self.win_for_surface(surface.wl_surface()) {
            // Told it has the whole screen already, and kept in its place: it stays there.
            if self.wins.get(&id).is_some_and(|w| w.windowed) {
                return;
            }
            // On the display it asks for, when it names one.
            if let Some(o) = output.as_ref().and_then(Output::from_resource) {
                let wins = &self.wins;
                self.layout.move_window_to_monitor(id, &o.name(), &|w| State::chosen_size(wins, w));
            }
            self.layout.set_fullscreen(id, Some(true));
            self.queue_redraw();
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.win_for_surface(surface.wl_surface()) {
            if let Some(w) = self.wins.get_mut(&id) {
                w.windowed = false;
            }
            self.layout.set_fullscreen(id, Some(false));
            self.queue_redraw();
        }
    }

    // A window's own title bar dragged, or its own edge: for a floating window that is the
    // same as holding it with Mod. A window in a column stays in its column.
    fn move_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, _serial: Serial) {
        if let Some(id) = self.win_for_surface(surface.wl_surface()) {
            crate::float::start(self, id, false, None, false);
        }
    }

    fn resize_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, _serial: Serial, edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge as E;
        let corner = (matches!(edges, E::Left | E::TopLeft | E::BottomLeft), matches!(edges, E::Top | E::TopLeft | E::TopRight));
        if let Some(id) = self.win_for_surface(surface.wl_surface()) {
            crate::float::start(self, id, true, Some(corner), false);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        // A column is as tall as the screen already; answer so the client isn't left waiting.
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.unmapped.retain(|w| w.toplevel() != Some(&surface));
        if let Some(id) = self.win_for_surface(surface.wl_surface()) {
            self.unmap_window(id);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.names_changed(&surface);
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.names_changed(&surface);
    }
}

impl State {
    fn names_changed(&mut self, surface: &ToplevelSurface) {
        let Some(win) = self.win_for_surface(surface.wl_surface()).and_then(|id| self.wins.get(&id)) else { return };
        let (title, app_id) = with_states(surface.wl_surface(), |states| {
            let d = states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
            (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
        });
        win.foreign.send_title(&title);
        win.foreign.send_app_id(&app_id);
        win.foreign.send_done();
        // A new name may fit other lines of the settings: how it looks follows.
        let rule = self.config.window_rule(&app_id, &title);
        if let Some(id) = self.win_for_surface(surface.wl_surface())
            && let Some(w) = self.wins.get_mut(&id)
        {
            self.layout.set_bar_rule(id, rule.title_bar);
            w.rule = rule;
        }
        self.queue_redraw();
    }

    /// Keep a menu on the display its window is on.
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        let mut target = None;
        if let Some(id) = self.win_for_surface(&root) {
            let win = &self.wins[&id];
            for output in &self.outputs {
                if let Some(t) = self.tiles(output).iter().find(|t| t.id == id) {
                    let size = State::output_size(output);
                    // In the window's own coordinates.
                    let mut r = Rectangle::from_size(size);
                    r.loc -= smithay::utils::Point::from((t.rect.x, t.rect.y));
                    r.loc += win.window.geometry().loc;
                    target = Some(r);
                }
            }
        } else {
            for output in &self.outputs {
                let map = layer_map_for_output(output);
                if let Some(layer) = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL) {
                    let mut r = Rectangle::from_size(State::output_size(output));
                    r.loc -= map.layer_geometry(layer).map(|g| g.loc).unwrap_or_default();
                    target = Some(r);
                }
            }
        }
        let Some(mut target) = target else { return };
        target.loc -= get_popup_toplevel_coords(&kind);
        popup.with_pending_state(|state| state.geometry = state.positioner.get_unconstrained_geometry(target));
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn decoration_destroyed(&mut self, _toplevel: ToplevelSurface) {}

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, wl_output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let Some(output) = wl_output.as_ref().and_then(Output::from_resource).or_else(|| self.active_output().cloned()) else { return };
        send_scale(surface.wl_surface(), &output);
        let mut map = layer_map_for_output(&output);
        let _ = map.map_layer(&LayerSurface::new(surface, namespace));
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for output in &self.outputs {
            let mut map = layer_map_for_output(output);
            let found = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            if let Some(layer) = found {
                map.unmap_layer(&layer);
            }
        }
        if self.layer_focus.as_ref().is_some_and(|l| l.layer_surface() == &surface) {
            self.layer_focus = None;
        }
        self.queue_redraw();
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&popup);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.queue_redraw();
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.dh.get_client(s.id()).ok());
        set_data_device_focus(&self.dh, seat, client.clone());
        set_primary_focus(&self.dh, seat, client);
    }
}

impl TabletSeatHandler for State {
    type ToolFocus = WlSurface;
}
impl PointerConstraintsHandler for State {
    // A lock or a fence asked for while the pointer is already on the window holds at once.
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &smithay::input::pointer::PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            self.constrain_pointer();
        }
    }
}

impl smithay::wayland::text_input::TextInputActivation for State {}

impl smithay::wayland::input_method::InputMethodHandler for State {
    // The list of candidates an input method shows by the text being typed.
    fn new_popup(&mut self, surface: smithay::wayland::input_method::PopupSurface) {
        let _ = self.popups.track_popup(PopupKind::from(surface));
        self.queue_redraw();
    }

    fn dismiss_popup(&mut self, surface: smithay::wayland::input_method::PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = smithay::desktop::PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
        self.queue_redraw();
    }

    fn popup_repositioned(&mut self, _: smithay::wayland::input_method::PopupSurface) {
        self.queue_redraw();
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, smithay::utils::Logical> {
        self.win_for_surface(parent).and_then(|id| self.wins.get(&id)).map(|w| w.window.geometry()).unwrap_or_default()
    }
}

impl SelectionHandler for State {
    /// A screenshot on the clipboard, as a PNG.
    type SelectionUserData = std::sync::Arc<[u8]>;

    fn send_selection(&mut self, _ty: smithay::wayland::selection::SelectionTarget, _mime_type: String, fd: std::os::fd::OwnedFd, _seat: Seat<Self>, data: &Self::SelectionUserData) {
        // A reader may take its time, or leave half way: not on the compositor's own thread.
        let data = data.clone();
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut std::fs::File::from(fd), &data);
        });
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for State {}

impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: Source>(&mut self, source: S, _icon: Option<WlSurface>, seat: Seat<Self>, serial: Serial, type_: GrabType) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let Some(start_data) = ptr.grab_start_data() else {
                    source.cancel();
                    return;
                };
                let grab = DnDGrab::new_pointer(&self.dh, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => source.cancel(),
        }
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl DataControlHandler for State {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

impl OutputHandler for State {}

impl smithay::wayland::virtual_keyboard::VirtualKeyboardHandler for State {
    fn process_virtual_keyboard_event(&mut self, event: smithay::backend::input::InputEvent<smithay::wayland::virtual_keyboard::VirtualKeyboardBackend>) {
        use smithay::backend::input::InputEvent;
        use smithay::wayland::virtual_keyboard::VirtualKeyboardSpecialEvent as Special;
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        match event {
            // Its key numbers mean something only in its own layout: use that while it types.
            InputEvent::DeviceAdded { device } | InputEvent::Special(Special::KeymapChanged { device }) => {
                if let Some(keymap) = device.keymap()
                    && keyboard.set_keymap_from_keymap_file(self, keymap).is_ok()
                {
                    self.borrowed_keymap = true;
                }
            }
            // And the settings' layout again when it has gone.
            InputEvent::DeviceRemoved { .. } => self.restore_keymap(),
            InputEvent::Special(Special::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. }) => {
                keyboard.with_xkb_state(self, |mut context| context.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group));
            }
            event @ InputEvent::Keyboard { .. } => self.process_input(event, None),
            _ => {}
        }
    }
}

/// A pointer made by a program (a remote desktop, a check): handled like a real one.
impl smithay::wayland::virtual_pointer::VirtualPointerHandler for State {
    fn process_virtual_pointer_event(&mut self, event: smithay::backend::input::InputEvent<smithay::wayland::virtual_pointer::VirtualPointerBackend>) {
        self.process_input(event, None);
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // From here on nothing but lock surfaces is drawn, so the lock holds at once.
        self.lock.locked = true;
        self.lock.surfaces.clear();
        confirmation.lock();
        self.queue_redraw();
    }

    fn unlock(&mut self) {
        self.lock.locked = false;
        self.lock.surfaces.clear();
        self.queue_redraw();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else { return };
        let size = State::output_size(&output);
        surface.with_pending_state(|s| s.size = Some((size.w as u32, size.h as u32).into()));
        surface.send_configure();
        send_scale(surface.wl_surface(), &output);
        self.lock.surfaces.retain(|(o, _)| *o != output);
        self.lock.surfaces.push((output, surface));
        self.queue_redraw();
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}

impl smithay::wayland::background_effect::ExtBackgroundEffectHandler for State {
    fn set_blur_region(&mut self, _: WlSurface, _: smithay::wayland::compositor::RegionAttributes) {
        self.queue_redraw();
    }
    fn unset_blur_region(&mut self, _: WlSurface) {
        self.queue_redraw();
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.push(surface);
        crate::idle_hold::update(self);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.retain(|s| *s != surface);
        crate::idle_hold::update(self);
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(&mut self, _token: XdgActivationToken, token_data: XdgActivationTokenData, surface: WlSurface) {
        // An app asked for one of its windows to come forward (a notification clicked, a file
        // opened in a running app). Recent requests only, so nothing steals the focus later.
        if token_data.timestamp.elapsed().as_secs() < 10
            && let Some(id) = self.win_for_surface(&surface)
        {
            self.layout.focus_window(id);
            self.layer_focus = None;
            self.queue_redraw();
        }
    }
}

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let root = root_surface(&surface);
        let output = self.win_for_surface(&root).and_then(|id| self.wins[&id].output.clone()).and_then(|n| self.output_named(&n).cloned()).or_else(|| self.active_output().cloned());
        if let Some(output) = output {
            send_scale(&surface, &output);
        }
    }
}

smithay::delegate_dispatch2!(State);
