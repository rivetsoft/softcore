//! Input that isn't a key, a button or plain pointer movement: fingers on a touchpad, a
//! touchscreen, a drawing tablet, and a pointer a game has locked in place.

use smithay::backend::input::{
    AbsolutePositionEvent, Device, DeviceCapability, Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _, InputBackend, InputEvent, PointerMotionEvent, ProximityState, TabletToolButtonEvent, TabletToolEvent,
    TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState, TouchEvent,
};
use smithay::input::pointer::{GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent, RelativeMotionEvent};
use smithay::input::tablet::{self, TabletDescriptor, TabletSeatTrait};
use smithay::input::touch::{DownEvent, UpEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};

use crate::config::Action;
use crate::state::State;
use crate::ui::Panel;

/// Fingers swiping on the touchpad, when the swipe is Softcore's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Swipe {
    /// Three fingers: the windows or the workspaces follow them.
    Scroll,
    /// Four: up for the overview, down to leave it. How far they have gone, and whether
    /// that has already been acted on.
    Overview(f64, bool),
}

/// How far four fingers go before the overview answers.
const OVERVIEW_SWIPE: f64 = 60.0;

impl State {
    /// A point on a touchscreen or tablet, on the display it belongs to: the laptop's own
    /// screen when there is one, else the first.
    fn device_position<I: InputBackend, E: AbsolutePositionEvent<I>>(&self, event: &E) -> Option<Point<f64, Logical>> {
        let output = self.outputs.iter().find(|o| o.name().starts_with("eDP")).or(self.outputs.first())?;
        let g = State::output_geometry(output);
        Some(event.position_transformed(g.size) + g.loc.to_f64())
    }

    /// The pointer moved by this much. A program that has locked it gets the movement and
    /// the pointer stays put; one that has confined it keeps it inside its window.
    pub fn pointer_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
        let pointer = self.seat.get_pointer().unwrap();
        let under = self.surface_under(self.pointer_pos);
        let (mut locked, mut confined, mut region) = (false, false, None);
        if self.drag.is_none()
            && let Some((surface, at)) = &under
        {
            let inside = (self.pointer_pos - *at).to_i32_round();
            with_pointer_constraint(surface, &pointer, |c| {
                let Some(c) = c.filter(|c| c.is_active()) else { return };
                if !c.region().is_none_or(|r| r.contains(inside)) {
                    return;
                }
                match &*c {
                    PointerConstraint::Locked(_) => locked = true,
                    PointerConstraint::Confined(conf) => {
                        confined = true;
                        region = conf.region().cloned();
                    }
                }
            });
        }
        pointer.relative_motion(self, under.clone(), &RelativeMotionEvent { delta: event.delta(), delta_unaccel: event.delta_unaccel(), time: event.time() });
        if locked {
            pointer.frame(self);
            return;
        }
        let mut pos = self.pointer_pos + event.delta();
        // Keep the pointer on a display: the nearest edge of the one it was on.
        if self.output_at(pos).is_none()
            && let Some(g) = self.output_at(self.pointer_pos).or(self.outputs.first()).map(State::output_geometry)
        {
            pos.x = pos.x.clamp(g.loc.x as f64, (g.loc.x + g.size.w - 1) as f64);
            pos.y = pos.y.clamp(g.loc.y as f64, (g.loc.y + g.size.h - 1) as f64);
        }
        if confined && let Some((surface, at)) = &under {
            // Each way on its own, so it slides along the edge it meets.
            if let Some(r) = &region {
                if !r.contains((Point::from((pos.x, self.pointer_pos.y)) - *at).to_i32_round()) {
                    pos.x = self.pointer_pos.x;
                }
                if !r.contains((Point::from((self.pointer_pos.x, pos.y)) - *at).to_i32_round()) {
                    pos.y = self.pointer_pos.y;
                }
            }
            // Off its window: as far as it can go one way or the other, or nowhere.
            let on = |state: &State, p: Point<f64, Logical>| state.surface_under(p).map(|(s, _)| s).as_ref() == Some(surface);
            if !on(self, pos) {
                let (across, down) = (Point::from((pos.x, self.pointer_pos.y)), Point::from((self.pointer_pos.x, pos.y)));
                pos = if on(self, across) {
                    across
                } else if on(self, down) {
                    down
                } else {
                    pointer.frame(self);
                    return;
                };
            }
        }
        self.pointer_moved(pos, event.time());
    }

    /// A lock or a fence a program asked for takes hold once the pointer is on its window.
    pub fn constrain_pointer(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let Some((surface, at)) = self.surface_under(self.pointer_pos) else { return };
        let inside = (self.pointer_pos - at).to_i32_round();
        with_pointer_constraint(&surface, &pointer, |c| {
            if let Some(c) = c.filter(|c| !c.is_active())
                && c.region().is_none_or(|r| r.contains(inside))
            {
                c.activate();
            }
        });
    }

    /// Everything a device sends that `process_input` doesn't handle itself.
    pub fn device_input<I: InputBackend>(&mut self, event: InputEvent<I>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        match event {
            InputEvent::DeviceAdded { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    self.seat.tablet_seat().add_wp_tablet(&self.dh, &TabletDescriptor::from(&device));
                }
                if device.has_capability(DeviceCapability::Touch) && self.seat.get_touch().is_none() {
                    self.seat.add_touch();
                }
            }
            InputEvent::DeviceRemoved { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    let tablets = self.seat.tablet_seat();
                    tablets.remove_tablet(&TabletDescriptor::from(&device));
                    if tablets.count_tablets() == 0 {
                        tablets.clear_tools();
                    }
                }
            }

            // ---- fingers on the touchpad ----
            // Three and four fingers are Softcore's; the rest is the program's under the
            // pointer.
            InputEvent::GestureSwipeBegin { event, .. } => {
                let free = !self.lock.locked && self.ui.panel == Panel::None;
                self.swipe = match event.fingers() {
                    3 if free => {
                        self.layout.swipe_begin();
                        Some(Swipe::Scroll)
                    }
                    4 if free => Some(Swipe::Overview(0.0, false)),
                    _ => None,
                };
                if self.swipe.is_none() {
                    pointer.gesture_swipe_begin(self, &GestureSwipeBeginEvent { serial, time: event.time(), fingers: event.fingers() });
                }
            }
            InputEvent::GestureSwipeUpdate { event, .. } => match self.swipe {
                Some(Swipe::Scroll) => {
                    let d = event.delta();
                    self.layout.swipe_update(d.x, d.y);
                    self.queue_redraw();
                }
                Some(Swipe::Overview(gone, done)) => {
                    let gone = gone + event.delta().y;
                    let act = !done && gone.abs() >= OVERVIEW_SWIPE;
                    self.swipe = Some(Swipe::Overview(gone, done || act));
                    if act {
                        self.do_action(Action::Overview(Some(gone < 0.0)));
                    }
                }
                None => pointer.gesture_swipe_update(self, &GestureSwipeUpdateEvent { time: event.time(), delta: event.delta() }),
            },
            InputEvent::GestureSwipeEnd { event, .. } => match self.swipe.take() {
                Some(Swipe::Scroll) => {
                    self.layout.swipe_end();
                    self.queue_redraw();
                }
                Some(Swipe::Overview(..)) => {}
                None => pointer.gesture_swipe_end(self, &GestureSwipeEndEvent { serial, time: event.time(), cancelled: event.cancelled() }),
            },
            InputEvent::GesturePinchBegin { event, .. } => pointer.gesture_pinch_begin(self, &GesturePinchBeginEvent { serial, time: event.time(), fingers: event.fingers() }),
            InputEvent::GesturePinchUpdate { event, .. } => pointer.gesture_pinch_update(self, &GesturePinchUpdateEvent { time: event.time(), delta: event.delta(), scale: event.scale(), rotation: event.rotation() }),
            InputEvent::GesturePinchEnd { event, .. } => pointer.gesture_pinch_end(self, &GesturePinchEndEvent { serial, time: event.time(), cancelled: event.cancelled() }),
            InputEvent::GestureHoldBegin { event, .. } => pointer.gesture_hold_begin(self, &GestureHoldBeginEvent { serial, time: event.time(), fingers: event.fingers() }),
            InputEvent::GestureHoldEnd { event, .. } => pointer.gesture_hold_end(self, &GestureHoldEndEvent { serial, time: event.time(), cancelled: event.cancelled() }),

            // ---- a touchscreen ----
            InputEvent::TouchDown { event, .. } => {
                let (Some(touch), Some(pos)) = (self.seat.get_touch(), self.device_position(&event)) else { return };
                if !self.lock.locked {
                    // A finger put down gives the keyboard to what it lands on, as a click does.
                    self.touched(pos);
                }
                let under = self.surface_under(pos);
                touch.down(self, under, &DownEvent { slot: event.slot(), location: pos, serial, time: event.time() });
            }
            InputEvent::TouchMotion { event, .. } => {
                let (Some(touch), Some(pos)) = (self.seat.get_touch(), self.device_position(&event)) else { return };
                let under = self.surface_under(pos);
                touch.motion(self, under, &smithay::input::touch::MotionEvent { slot: event.slot(), location: pos, time: event.time() });
            }
            InputEvent::TouchUp { event, .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.up(self, &UpEvent { slot: event.slot(), serial, time: event.time() });
                }
            }
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.frame(self);
                }
            }
            InputEvent::TouchCancel { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.cancel(self);
                }
            }

            // ---- a drawing tablet ----
            // The pointer goes where the pen is, so there is something to see, and the
            // program gets the pen itself with its pressure and tilt.
            InputEvent::TabletToolAxis { event, .. } => {
                let Some(pos) = self.device_position(&event) else { return };
                let time = event.time();
                let under = self.pen_moved(pos, time);
                if let Some(tool) = self.seat.tablet_seat().get_tool(&event.tool()) {
                    tool.axis(self, pen_axes::<I, _>(&event));
                    tool.motion(self, under, &tablet::tool::MotionEvent { location: pos, serial, time });
                    tool.frame(self, time);
                }
            }
            InputEvent::TabletToolProximity { event, .. } => {
                let Some(pos) = self.device_position(&event) else { return };
                let time = event.time();
                let under = self.pen_moved(pos, time);
                let tablets = self.seat.tablet_seat();
                let Some(tablet) = tablets.get_tablet(&TabletDescriptor::from(&event.device())) else { return };
                let desc = event.tool();
                let dh = self.dh.clone();
                let tool = tablets.get_tool(&desc).unwrap_or_else(|| tablets.add_wp_tool(self, &dh, &desc));
                match event.state() {
                    ProximityState::In => tool.proximity_in(self, under, tablet, &tablet::tool::ProximityInEvent { location: pos, axis: Some(pen_axes::<I, _>(&event)), serial, time }),
                    ProximityState::Out => tool.proximity_out(self, &tablet::tool::ProximityOutEvent { serial, time }),
                }
                tool.frame(self, time);
            }
            InputEvent::TabletToolTip { event, .. } => {
                let Some(tool) = self.seat.tablet_seat().get_tool(&event.tool()) else { return };
                let time = event.time();
                match event.tip_state() {
                    TabletToolTipState::Down => {
                        if !self.lock.locked {
                            self.touched(self.pointer_pos);
                        }
                        tool.down(self, &tablet::tool::DownEvent { serial, time });
                    }
                    TabletToolTipState::Up => tool.up(self, &tablet::tool::UpEvent { serial, time }),
                }
                tool.frame(self, time);
            }
            InputEvent::TabletToolButton { event, .. } => {
                let Some(tool) = self.seat.tablet_seat().get_tool(&event.tool()) else { return };
                let time = event.time();
                tool.button(self, &tablet::tool::ButtonEvent { serial, button: event.button(), state: event.button_state(), time });
                tool.frame(self, time);
            }
            _ => {}
        }
    }

    /// The pen is over this point: the pointer follows it. Says what is under it.
    fn pen_moved(&mut self, pos: Point<f64, Logical>, time: smithay::backend::input::InputTime) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.pointer_pos = pos;
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(self, under.clone(), &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
        self.queue_redraw();
        under
    }
}

/// What a pen says about itself with a movement, where it has changed.
fn pen_axes<I: InputBackend, E: TabletToolEvent<I>>(e: &E) -> tablet::tool::AxisFrame {
    tablet::tool::AxisFrame {
        pressure: e.pressure_has_changed().then(|| e.pressure()),
        distance: e.distance_has_changed().then(|| e.distance()),
        tilt: e.tilt_has_changed().then(|| e.tilt()),
        rotation: e.rotation_has_changed().then(|| e.rotation()),
        slider: e.slider_has_changed().then(|| e.slider_position()),
        wheel: e.wheel_has_changed().then(|| (e.wheel_delta(), e.wheel_delta_discrete())),
    }
}
