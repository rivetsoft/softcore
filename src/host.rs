//! In a window of another desktop: asking that desktop for its key bindings.
//!
//! A desktop keeps its own bindings from its windows (Super and a key is its to act on),
//! unless a window asks for them: a remote desktop does, a virtual machine does, and so does
//! a Softcore that is being tried out with Mod as Super. The desktop decides: niri gives
//! them while the window has the focus and takes them back with its own key (Mod+Escape).

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1 as Manager;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1 as Inhibitor;

struct Host;

/// Held for as long as the bindings are wanted.
pub struct Keys {
    queue: EventQueue<Host>,
    _inhibitor: Inhibitor,
}

impl Keys {
    /// What the desktop has said since last time: nothing Softcore acts on, but it isn't
    /// left to pile up.
    pub fn tidy(&mut self) {
        let _ = self.queue.dispatch_pending(&mut Host);
    }
}

macro_rules! quiet {
    ($($what:ty: $data:ty),*) => {$(
        impl Dispatch<$what, $data> for Host {
            fn event(_: &mut Self, _: &$what, _: <$what as Proxy>::Event, _: &$data, _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
quiet!(wl_registry::WlRegistry: GlobalListContents, WlSeat: (), Manager: (), Inhibitor: ());

/// Ask the desktop this window is on for its key bindings. None when it isn't a Wayland
/// desktop, or has no way to ask.
pub fn ask_for_keys(window: &(impl HasDisplayHandle + HasWindowHandle + ?Sized)) -> Option<Keys> {
    let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else { return None };
    let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else { return None };
    // The window's own connection, with a queue of Softcore's on it.
    let conn = Connection::from_backend(unsafe { wayland_backend::client::Backend::from_foreign_display(display.display.as_ptr().cast()) });
    let (globals, mut queue) = registry_queue_init::<Host>(&conn).ok()?;
    let qh = queue.handle();
    let manager: Manager = globals.bind(&qh, 1..=1, ()).ok()?;
    let seat: WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
    let id = unsafe { wayland_backend::client::ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }.ok()?;
    let surface = WlSurface::from_id(&conn, id).ok()?;
    let inhibitor = manager.inhibit_shortcuts(&surface, &seat, &qh, ());
    let _ = queue.roundtrip(&mut Host);
    Some(Keys { queue, _inhibitor: inhibitor })
}
