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
        bind_matches(b, self.mod_is_alt(), (mods.ctrl, mods.shift, mods.alt, mods.logo), sym)
    }

    /// What a key press means to Softcore itself, if anything. `Some(None)`: taken, with
    /// nothing more to do.
    fn key_pressed(&mut self, mods: &ModifiersState, sym: Keysym, modified: Keysym) -> Option<Option<Action>> {
        // Ctrl+Alt+F1…F12: another TTY, whatever else is going on. The keyboard layout turns
        // those into keys of their own, which only the modified key shows.
        let vt = modified.raw().wrapping_sub(Keysym::XF86_Switch_VT_1.raw());
        if vt < 12 {
            return Some(Some(Action::ChangeVt(vt as i32 + 1)));
        }
        if self.ui.panel == Panel::Share && !self.lock.locked {
            let chosen = self.picker.as_ref().map(|p| p.selected);
            match sym {
                Keysym::Left | Keysym::Up => crate::picker::select(self, -1),
                Keysym::Right | Keysym::Down | Keysym::Tab => crate::picker::select(self, 1),
                Keysym::Return | Keysym::KP_Enter => crate::picker::finish(self, chosen),
                Keysym::Escape => crate::picker::finish(self, None),
                _ => {}
            }
            return Some(None);
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
        // A window given every key gets Softcore's bindings too, all but the way back.
        let all = !locked && self.all_keys.is_some() && self.all_keys == self.layout.active_window() && self.layer_focus.is_none();
        let found = self.config.binds.iter().find(|b| (!locked || b.locked) && (!all || b.action == Action::ToggleShortcutsInhibit) && self.bind_matches(b, mods, sym)).map(|b| b.action.clone());
        if found.is_none() && self.overview.on && !locked {
            // The overview has the keyboard: arrows go about it, Return and Escape leave it,
            // and nothing is typed into the window it will leave on.
            if sym.is_modifier_key() {
                return None;
            }
            use crate::layout::Dir;
            let dir = match sym {
                Keysym::Left => Some(Dir::Left),
                Keysym::Right => Some(Dir::Right),
                Keysym::Up => Some(Dir::Up),
                Keysym::Down => Some(Dir::Down),
                _ => None,
            };
            if let Some(d) = dir {
                self.layout.focus_or_workspace(d);
                self.queue_redraw();
            }
            return Some(matches!(sym, Keysym::Escape | Keysym::Return | Keysym::KP_Enter).then_some(Action::Overview(Some(false))));
        }
        found.map(Some)
    }

    pub fn process_input<I: InputBackend>(&mut self, event: InputEvent<I>, output: Option<&Output>) {
        self.idle_notifier_state.notify_activity(&self.seat);
        // Any key or movement brings blanked displays back.
        if self.monitors_off && matches!(event, InputEvent::Keyboard { .. } | InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. } | InputEvent::PointerButton { .. }) {
            self.monitors_off = false;
            self.queue_redraw();
        }
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
                    match state.key_pressed(mods, sym, handle.modified_sym()) {
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
                let pressed = event.state() == ButtonState::Pressed;
                // A floating window held by the pointer: any button let go lets go of it.
                if !pressed && let Some(d) = &self.drag {
                    let quiet = d.quiet;
                    crate::float::end(self);
                    if quiet {
                        return;
                    }
                }
                if !pressed && std::mem::take(&mut self.overview.eat_release) {
                    return;
                }
                if pressed && self.overview.on && !self.lock.locked && self.ui.panel == Panel::None && self.surface_under(self.pointer_pos).is_none() {
                    crate::overview::clicked(self, self.pointer_pos);
                    return;
                }
                if pressed && !pointer.is_grabbed() && !self.lock.locked {
                    self.clicked(self.pointer_pos);
                    // Mod and a button on a floating window: left moves it, right resizes it.
                    let mods = self.seat.get_keyboard().unwrap().modifier_state();
                    let held = if self.mod_is_alt() { mods.alt } else { mods.logo };
                    let button = event.button_code();
                    if held
                        && (button == 0x110 || button == 0x111)
                        && self.ui.panel == Panel::None
                        && let Some(id) = self.window_under(self.pointer_pos)
                        && crate::float::start(self, id, button == 0x111, None, true)
                    {
                        return;
                    }
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
        if self.drag.is_some() {
            // The window follows the pointer; nothing under it hears of the movement.
            crate::float::moved(self);
            self.queue_redraw();
            return;
        }
        crate::picker::hover(self, pos);
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(self, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
        self.queue_redraw();
    }

    /// A click gives the keyboard to what was clicked: a window, or a panel that takes keys.
    fn clicked(&mut self, pos: Point<f64, Logical>) {
        if self.ui.panel == Panel::Share {
            if let Some(i) = crate::picker::item_at(self, pos) {
                crate::picker::finish(self, Some(i));
            }
            return;
        }
        if let Some(name) = self.output_at(pos).map(|o| o.name()) {
            self.layout.set_active_monitor(&name);
        }
        if let Some(id) = self.window_under(pos) {
            self.layout.focus_window(id);
            self.layer_focus = None;
        } else {
            self.layer_focus = self.layer_under(pos).filter(|l| l.cached_state().keyboard_interactivity != KeyboardInteractivity::None);
        }
        self.update_focus();
        self.queue_redraw();
    }

    fn wheel<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let mods = self.seat.get_keyboard().map(|k| k.modifier_state()).unwrap_or_default();
        let mod_down = if self.mod_is_alt() { mods.alt } else { mods.logo };
        if (mod_down || self.overview.on) && !self.lock.locked {
            // Mod and the wheel: through the workspaces; with Shift, along the columns.
            let v = event.amount_v120(Axis::Vertical).unwrap_or(0.0);
            if v != 0.0 {
                let by = if v > 0.0 { 1 } else { -1 };
                let action = match (mods.shift, by) {
                    (false, by) => Action::FocusWorkspaceBy(by),
                    (true, by) => Action::Focus(self.layout.along(by == 1)),
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

/// Whether a binding is the key pressed, with exactly its modifiers held (Ctrl, Shift, Alt,
/// Super). `alt_mod`: Mod is Alt, not Super.
pub fn bind_matches(b: &Bind, alt_mod: bool, held: (bool, bool, bool, bool), sym: Keysym) -> bool {
    let want_alt = b.mods.alt || (b.mods.modkey && alt_mod);
    let want_logo = b.mods.logo || (b.mods.modkey && !alt_mod);
    held == (b.mods.ctrl, b.mods.shift, want_alt, want_logo) && key_sym(&b.key) == Some(sym)
}

/// The key a binding names, if there is such a key.
pub fn key_sym(name: &str) -> Option<Keysym> {
    let sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
    (sym != Keysym::NoSymbol).then_some(sym)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn every_default_binding_names_a_real_key() {
        let c = Config::default();
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        for b in &c.binds {
            assert!(key_sym(&b.key).is_some(), "{} names no key", b.shown);
        }
    }

    #[test]
    fn mod_is_super_or_alt_and_modifiers_must_match_exactly() {
        let c = Config::default();
        let find = |alt_mod, held, sym| c.binds.iter().find(|b| bind_matches(b, alt_mod, held, sym)).map(|b| b.action.clone());
        // Ctrl, Shift, Alt, Super.
        assert_eq!(find(false, (false, false, false, true), Keysym::q), Some(Action::CloseWindow));
        assert_eq!(find(true, (false, false, true, false), Keysym::q), Some(Action::CloseWindow));
        assert_eq!(find(true, (false, false, false, true), Keysym::q), None);
        assert_eq!(find(false, (false, false, false, false), Keysym::q), None);
        // Shift held: a different binding, and the key is still "f", not "F".
        assert_eq!(find(false, (false, true, false, true), Keysym::f), Some(Action::FullscreenWindow));
        assert_eq!(find(false, (false, false, false, true), Keysym::f), Some(Action::MaximizeColumn));
        assert_eq!(find(false, (true, false, false, true), Keysym::_3), Some(Action::MoveToWorkspace { idx: 2, column: true }));
        assert_eq!(find(false, (false, false, false, true), Keysym::bracketleft), Some(Action::ConsumeOrExpel(crate::layout::Dir::Left)));
    }
}
