//! Keys, the pointer and the wheel: what is Softcore's own (bindings, the open panel), and
//! what goes on to the focused client.

use smithay::backend::input::{AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent};
use smithay::input::keyboard::{FilterResult, Keysym, ModifiersState, xkb};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::output::Output;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;

use crate::config::{Action, Bind};
use crate::state::State;
use crate::ui::Panel;

impl State {
    fn bind_matches(&self, b: &Bind, mods: &ModifiersState, sym: Keysym) -> bool {
        let alt_mod = self.mod_is_alt();
        let want_alt = b.mods.alt || (b.mods.modkey && alt_mod);
        let want_logo = b.mods.logo || (b.mods.modkey && !alt_mod);
        (mods.ctrl, mods.shift, mods.alt, mods.logo) == (b.mods.ctrl, b.mods.shift, want_alt, want_logo) && xkb::keysym_from_name(&b.key, xkb::KEYSYM_CASE_INSENSITIVE) == sym
    }

    /// What a key press means to Softcore itself, if anything. `Some(None)`: taken, with
    /// nothing more to do.
    fn key_pressed(&mut self, mods: &ModifiersState, sym: Keysym) -> Option<Option<Action>> {
        // Ctrl+Alt+F1…F12: another TTY, whatever else is going on.
        let vt = sym.raw().wrapping_sub(Keysym::XF86_Switch_VT_1.raw());
        if vt < 12 {
            return Some(Some(Action::ChangeVt(vt as i32 + 1)));
        }
        if self.ui.panel != Panel::None && !self.lock.locked {
            // An open panel has the keyboard to itself.
            let quit = self.ui.panel == Panel::Quit && matches!(sym, Keysym::Return | Keysym::KP_Enter);
            if quit || matches!(sym, Keysym::Escape | Keysym::Return | Keysym::KP_Enter) {
                self.ui.show(Panel::None);
                self.queue_redraw();
            }
            return Some(quit.then_some(Action::Quit { confirm: false }));
        }
        let locked = self.lock.locked;
        let found = self.config.binds.iter().find(|b| (!locked || b.locked) && self.bind_matches(b, mods, sym)).map(|b| b.action.clone());
        found.map(Some)
    }

    pub fn process_input<I: InputBackend>(&mut self, event: InputEvent<I>, output: Option<&Output>) {
        self.idle_notifier_state.notify_activity(&self.seat);
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time(&event);
                let code = event.key_code();
                let pressed = event.state() == KeyState::Pressed;
                let keyboard = self.seat.get_keyboard().unwrap();
                let action = keyboard.input(self, code, event.state(), serial, time, |state, mods, handle| {
                    let raw = code.raw();
                    if !pressed {
                        return match state.suppressed_keys.iter().position(|k| *k == raw) {
                            Some(i) => {
                                state.suppressed_keys.remove(i);
                                FilterResult::Intercept(None)
                            }
                            None => FilterResult::Forward,
                        };
                    }
                    // The key as it is without Shift, so Mod+Shift+1 is "1" and not "!".
                    let sym = handle.raw_latin_sym_or_raw_current_sym().unwrap_or_else(|| handle.modified_sym());
                    match state.key_pressed(mods, sym) {
                        Some(action) => {
                            state.suppressed_keys.push(raw);
                            FilterResult::Intercept(action)
                        }
                        None => FilterResult::Forward,
                    }
                });
                if let Some(Some(action)) = action {
                    self.do_action(action);
                }
            }
            InputEvent::PointerMotion { event, .. } => {
                let mut pos = self.pointer_pos + event.delta();
                // Keep the pointer on a display: the nearest edge of the one it was on.
                if self.output_at(pos).is_none()
                    && let Some(g) = self.output_at(self.pointer_pos).or(self.outputs.first()).map(State::output_geometry)
                {
                    pos.x = pos.x.clamp(g.loc.x as f64, (g.loc.x + g.size.w - 1) as f64);
                    pos.y = pos.y.clamp(g.loc.y as f64, (g.loc.y + g.size.h - 1) as f64);
                }
                self.pointer_moved(pos, event.time());
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(g) = output.or(self.outputs.first()).map(State::output_geometry) else { return };
                let pos = event.position_transformed(g.size) + g.loc.to_f64();
                self.pointer_moved(pos, event.time());
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() && !self.lock.locked {
                    self.clicked(self.pointer_pos);
                }
                pointer.button(self, &ButtonEvent { button: event.button_code(), state: event.state(), serial, time: event.time() });
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => self.wheel::<I>(event),
            _ => {}
        }
    }

    fn pointer_moved(&mut self, pos: Point<f64, Logical>, time: smithay::backend::input::InputTime) {
        self.pointer_pos = pos;
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(self, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
        self.queue_redraw();
    }

    /// A click gives the keyboard to what was clicked: a window, or a panel that takes keys.
    fn clicked(&mut self, pos: Point<f64, Logical>) {
        if let Some(name) = self.output_at(pos).map(|o| o.name()) {
            self.layout.set_active_monitor(&name);
        }
        if let Some(id) = self.window_under(pos) {
            self.layout.focus_window(id);
            self.layer_focus = None;
        } else {
            self.layer_focus = self.layer_under(pos).filter(|l| l.cached_state().keyboard_interactivity != KeyboardInteractivity::None);
        }
        self.sync_and_focus();
        self.queue_redraw();
    }

    fn sync_and_focus(&mut self) {
        self.update_focus();
    }

    fn wheel<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let mods = self.seat.get_keyboard().map(|k| k.modifier_state()).unwrap_or_default();
        let mod_down = if self.mod_is_alt() { mods.alt } else { mods.logo };
        if mod_down && !self.lock.locked {
            // Mod and the wheel: through the workspaces; with Shift, along the columns.
            let v = event.amount_v120(Axis::Vertical).unwrap_or(0.0);
            if v != 0.0 {
                let by = if v > 0.0 { 1 } else { -1 };
                let action = match (mods.shift, by) {
                    (false, by) => Action::FocusWorkspaceBy(by),
                    (true, 1) => Action::Focus(crate::layout::Dir::Right),
                    (true, _) => Action::Focus(crate::layout::Dir::Left),
                };
                self.do_action(action);
            }
            return;
        }
        let source = event.source();
        let mut frame = AxisFrame::new(event.time()).source(source);
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let amount = event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
            if amount != 0.0 {
                frame = frame.value(axis, amount);
                if let Some(d) = event.amount_v120(axis) {
                    frame = frame.v120(axis, d as i32);
                }
            } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                frame = frame.stop(axis);
            }
        }
        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}
