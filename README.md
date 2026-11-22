<img src="assets/icon.svg" width="64" height="64" alt="">

# Softcore

The scrolling-tiling Wayland compositor for the Rivet desktop, in Rust on
[smithay](https://github.com/Smithay/smithay). Windows sit in columns on a strip that runs off
both sides of the screen; each display has its own stack of workspaces, with an empty one
always last. It works the way [niri](https://github.com/YaLTeR/niri) does, and is written from
scratch: niri's behaviour was studied from the outside (what it advertises to clients, what
`niri msg` says), none of its code is here.

When a display is unplugged its workspaces move to another display, and go back to it
when it returns. With no display left they wait for the next one.

Softcore is early. It runs in a window of another desktop and with no screen at all, and both
are checked. Running on displays of its own is written and compiles, but has not been run on
hardware yet (see *What isn't built yet*).

## Build and run

Needs the usual compositor libraries with their headers: libseat, libinput, libudev, libgbm,
libxkbcommon, EGL.

```sh
cargo build --release
softcore                # from a desktop: opens in a window. From a TTY: takes the displays.
softcore --headless     # a display in memory, for checks and screenshots
softcore --spawn foot   # run something once started
```

In a window, Mod is Alt (the desktop outside keeps Super); on displays of its own it is Super.
To try a new build inside a running Softcore, start it from a terminal there (it opens in a
window) and press Mod+Shift+Escape on that window: it then gets every key, the outer
Softcore's bindings included, and its border turns the opposite colour to say so. The same
keys take them back. `mod=super` in the inner one's settings (`SOFTCORE_CONFIG=FILE`) makes it
answer to the keys it will have on displays of its own.

With `mod=super` a Softcore in a window also asks the desktop it is on for its key
bindings, as a remote desktop or a virtual machine does. niri gives them while the window has
the focus, and takes them back with its own key (Mod+Escape by default). A Softcore outside
doesn't answer that request yet: there, Mod+Shift+Escape does it by hand.
Mod+Shift+/ shows every key. The ones to know first: Mod+Return a terminal, Mod+Space the
launcher, Mod+Q close, Mod+arrows move the focus, Mod+Ctrl+arrows move the window, Mod+R the
next column width, Mod+Shift+Minus and Mod+Shift+Equal a shorter or taller window (it sits in
the middle of the height; Mod+Ctrl+R gives it all the height back), Mod+F a column as wide as the screen, Mod+Page Up and Down the workspaces, Mod+Shift+S puts a
window away in the scratchpad and Mod+S shows it floating over whatever is in view (and puts
it away again),
Mod+Shift+E leave.

## What it shares with Rivet

Softcore depends on the shared crates of [rivetsoft/apps](https://github.com/rivetsoft/apps),
and anything it draws itself goes through them:

- **`rivet-look`**: the border of the focused window is the Rivet accent, followed live with
  the desktop's colours (a new wallpaper, another palette or light for dark in Rivet Shell
  recolours the borders and panels at once: the files are watched); the other
  borders, the colour behind everything and the corner radius come from the same theme.
  Softcore's own panels (the keys, the question before leaving, the note about settings that
  couldn't be read) are egui in that theme.
- **`rivet-kit`**: those panels are painted in memory by its software renderer, the way Rivet
  Shell paints its own.
- **`rivet-xdg`**: the user's folders.

The crates are pinned to a commit in `Cargo.toml`. To build against a checkout of the apps
repository instead, copy `.cargo/config.toml.example` to `.cargo/config.toml` and set the path.

Rivet Shell runs under Softcore (panels, dock, wallpaper, launcher: they are layer-shell
surfaces). Its workspaces item, window title, dock and log out work once the shell knows
Softcore by name: a `Compositor::Softcore` that asks `softcore msg` what it asks `niri msg`,
since the answers have the same fields. Softcore puts itself on the PATH of what it starts,
so `softcore msg` is found wherever Softcore was started from. The shell's Displays page
and window look settings work too, through a file of their own that the settings include
(see *Displays*).

## Settings

One file, `~/.config/softcore/settings`, that you and a settings window both edit. It is
`key=value` lines like every Rivet app's:

```
# Look
gaps=8
border=3
border_active=#e66100

# Keys
bind.Mod+T=spawn foot
bind.Mod+E=

# Displays
output.DP-5=mode=2560x1440@59.951 scale=1.5 position=0,0
output.eDP-1=off

spawn=rivet-shell
env.MOZ_ENABLE_WAYLAND=1
```

What makes it work from both sides:

- **Every plain setting is one key with one typed value**, listed with its type, range,
  default, label and group in one table (`SETTINGS` in `crates/softcore-settings`). The reader
  checks values against that table, and a window can draw its controls from it: a whole
  number in a range is a slider, `true`/`false` a switch, a few words a menu, `#rrggbb` a
  colour well. `softcore settings describe` prints the table as JSON.
- **Things there are many of have a name in their key** (`bind.KEYS`, `output.NAME`,
  `env.NAME`), so each is one line that can be found and changed. An empty binding
  (`bind.Mod+E=`) takes a default away. `spawn` is a plain list, in order.
- **A change touches one line.** The writer (`SettingsFile`) puts the new value where the
  old one stood and leaves comments, blank lines, the order of things and keys it doesn't
  know alone. So a file tidied by hand stays tidy after a window changes something, and the
  window shows what was typed by hand.
- **Order doesn't matter** between different keys, and an empty value means "follow": the
  edges follow `gaps`, colours and the radius follow Rivet.
- **Softcore reads the file again the moment it changes**, whoever changed it.

```sh
softcore settings                    # every setting and its value; a star marks ones in the file
softcore settings set gaps 16        # checked first, then one line changed
softcore settings set bind.Mod+T spawn kitty
softcore settings unset gaps         # back to the default
softcore validate                    # what couldn't be read, without starting anything
```

Lines that can't be read are listed in a panel on screen and the rest still applies; keys
Softcore doesn't know are skipped, so a newer file still loads. `include=FILE` reads another
file in place for those who split theirs up; a change made through `SettingsFile` always goes
where it has the last word.

The crate has no compositor or toolkit in it, so Rivet Shell's settings window can depend on
it and edit this file directly, through the code that owns the format. That window isn't
built yet.

The plain settings: `gaps`, `gaps_out`, `border`, `radius`, `border_active`,
`border_inactive`, `border_active_to`, `border_inactive_to`, `border_angle`,
`inactive_opacity`, `shadows`, `shadow_size`, `shadow_offset`,
`shadow_opacity`, `shadow_color`, `blur`, `blur_strength`, `blur_layers`, `animations`,
`animation_speed`, `scroll`, `overview_zoom`, `tab_bar`, `column_widths`, `center_short_columns`, `scratchpad_size`, `default_width`, `theme`,
`accent`, `follow_desktop_colors`, `background`, `mod`, `default_binds`, `xkb_layout`,
`xkb_variant`, `xkb_options`, `repeat_delay`, `repeat_rate`, `tap_to_click`,
`natural_scroll`, `cursor_theme`, `cursor_size`. `softcore settings` says what each is.

A binding's value is an action, named as niri names them (`focus-column-left`,
`move-column-to-workspace 3`, `set-column-width +10%`, `set-window-height 60%`, …), with `allow-when-locked` before it
for keys that should work on the lock screen. `Mod` is Super on displays of its own and Alt
in a window. The actions and the default bindings are in `crates/softcore-settings/src/lib.rs`.

`title="…"` before the action says what the keys panel (Mod+Shift+/) shows for a binding, in
place of the action's own words, and `title=""` leaves it out of the panel:

```
bind.Mod+T=title="Terminal" spawn rivet-terminal
bind.Mod+Shift+P=title="" spawn some-script
```

## Scrolling up and down

`scroll=vertical` turns the layout a quarter turn: windows sit in rows one under the other,
each as wide as the screen, and the strip scrolls up and down. `scroll=auto` does that only
on a display taller than it is wide, which suits one turned on its side; `horizontal` is
the usual way. A display's own line can say otherwise (`output.HDMI-A-1=scroll=vertical`).
A change applies as the file is saved, and the windows slide to their new places.

A workspace can go its own way too. Mod+Shift+W (`toggle-workspace-scroll`) turns the one
in view the other way, and `set-workspace-scroll horizontal|vertical|auto|default` says
which; `default` is as its display does again. A workspace told this keeps it wherever it
goes, to another display and back, until it is empty and gone. Workspaces still sit beside
one another the way their display says. `softcore msg workspaces` gives each one's `scroll`.

Everything follows the turn. Up and down go from row to row and left and right between the
windows sharing a row; a column's width settings (`default_width`, `column_widths`,
`set-column-width`) are a row's height, and `set-window-height` is a window's width in its
row. Workspaces, which sit one under the other beside sideways strips, sit side by side
here. The keys and action names stay as they are: `focus-column-left` means left on the
screen, and `consume-or-expel-window-left` means towards the row before.

## Full screen

Mod+Shift+F (`fullscreen-window`) gives the focused window the whole display, over the
shell's panels, and a program can ask for the same (and for a display by name, which it
then moves to). It grows from where the window stood and shrinks back there. One window a
workspace has it at a time, and it ends when the focus goes to another window, by a key or
a click, or when a new window opens on that workspace: the focus is never on something out
of sight.

Mod+Ctrl+Shift+F (`toggle-windowed-fullscreen`, niri's name) tells the program it has the
whole screen while the window stays in its place: a video or a game drops its own bars and
buttons, in a column beside the others.

## Tabs

Mod+W (`toggle-column-tabbed-display`, niri's name for it) makes tabs of the focused
column's windows: each has the whole column, one shows, and a strip across the top has a
piece for each, the one showing brightest. Up and down go from tab to tab, and a click on a
piece of the strip shows its window; the keys that move a window up and down its column
change the order, and the ones that take a window into or out of a column add and remove
tabs. Mod+W again shows them all. `tab_bar` is the strip's thickness. Where windows scroll
up and down, a row's tabs are a strip down its left.

## The overview

Mod+O (`toggle-overview`; there are `open-overview` and `close-overview` too) draws
everything back: every workspace of each display at once, at half size, with the strips
running off past the edges of the screen as far as they go. `overview_zoom` says how small
(0.5 is half).

In it, a click on a window gives it the focus and a click on a workspace brings that one
into view, and either shuts the overview. The arrows move the focus, on to the next
workspace past the end of a column; the wheel goes through the workspaces, and with Shift
along the strip. Return or Escape shuts it where the focus is. The bindings all go on
working, so windows can be moved between workspaces with the view drawn back, but nothing
is typed into a window, and the windows hear nothing from the pointer. The shell's panels
stay as they are and can be used.

`softcore msg overview-state` says whether it is open, and the event stream says
`OverviewOpenedOrClosed` when that changes, both as niri does.

## Displays

One line a display, by the name `softcore msg outputs` gives it:

```
output.DP-5=mode=2560x1440@59.951 scale=1.5 position=0,0 transform=normal
output.HDMI-A-1=transform=90
output.eDP-1=off
```

`mode` is a resolution and, after `@`, a refresh rate (the nearest the display has is used);
`transform` is `normal`, `90`, `180`, `270`, or `flipped` with or without a turn
(`flipped-90`). Anything left out stays the display's own choice. A change applies as the
file is saved: a display whose resolution changes, or that is turned off or on, is taken
down and set up again, and its workspaces come straight back to it. A display turned off
is still listed by `softcore msg outputs`, so it can be turned on again.

Rivet Shell's Displays page and its window look settings write these lines, and the look's
own (`gaps`, `border`, `radius`, …), into `rivet-shell.settings` next to the settings file,
which includes it with one line at its end: `include=rivet-shell.settings`. What you write
after that line still wins.

## Borders

A border is one colour, or a gradient: give it a second colour to run to
(`border_active_to` for the focused window, `border_inactive_to` for the others) and the
way it runs, `border_angle` in degrees (0 upwards, 90 to the right, 180 downwards). The
first colour of each is still Rivet's unless the settings name one, so a gradient can start
from the accent of the day.

```
border=4
border_active_to=#00c8ff
border_angle=135
```

## Floating windows

Dialogs and windows of a fixed size float over the columns; Mod+V floats any window, or puts
it back. A floating window starts in the middle at the size it chose, and from there:

- Mod and the left button moves it, Mod and the right button resizes it from the nearest
  corner. Its own title bar and edges work too, when it draws them.
- The keys that move and size columns do the same to it: Mod+Ctrl+arrows move it,
  Mod+Minus and Mod+Equal its width, Mod+Shift+Minus and Mod+Shift+Equal its height,
  Mod+C back to the middle, Mod+Ctrl+R back to the size it chooses itself.
- It stays where it was put, at the size it was given, through being put in a column and
  floated again. Enough of it always stays on the display to take hold of.

A window shown from the scratchpad is a floating window, and all of this goes for it.

## Motion, shadows and blur

Windows fade in a little small and grow into place, and fade out the same way; a window
that changes column, width or height slides and grows to its new place, and its neighbours
slide to make room. The strip and the workspaces slide as before. `animation_speed` makes
all of it faster or slower (2 is twice as fast), and `animations=false` puts everything
where it belongs at once.

A window changing size on its way is never stretched: it is drawn as it is and cut off at
where it has got to. Growing shows more of it; shrinking wipes across how it looked before
it began. A program that is killed outright vanishes without fading: its last frame goes
with it.

Shadows have a size (larger is softer), an offset below the window, a strength and a
colour: `shadow_size`, `shadow_offset`, `shadow_opacity`, `shadow_color`.

What shows through a window that isn't solid (a see-through terminal, an unfocused window
with `inactive_opacity` below 1) is blurred: `blur`, and `blur_strength` from 1 to 10.
Behind a window that is the wallpaper, which is all a tiled window has behind it, so its
blur is made once and kept until the wallpaper changes. A floating window shows the blurred
wallpaper too, not the windows under it.

Shell surfaces get blur behind them when they ask for it (ext-background-effect, with the
part of themselves they want it behind), or when `blur_layers` names them. Behind those
everything is blurred, windows included, and made again only when something under them
changed. A surface named in `blur_layers` is blurred behind all of its rectangle, so name
only ones that fill theirs: none are named by default, since Rivet Shell's panel and dock
are see-through around their bars and a blurred band would cover the windows under them.

## Talking to it

```sh
softcore msg windows            # also: outputs, workspaces, focused-window, focused-output
softcore msg -j workspaces      # as one line of JSON
softcore msg action focus-workspace 2
softcore msg action close-window --id 4
softcore msg screenshot shot.png
softcore msg event-stream       # a line whenever workspaces, windows or the focus change
```

The socket is `$SOFTCORE_SOCKET`, set for everything Softcore starts. A request is one line, a
JSON array of words; the answer is `{"ok": …}` or `{"error": "…"}`.

## Sharing the screen

An app in a call, or a recorder, can share a display or one window:

1. The app asks xdg-desktop-portal, which asks Softcore's own service (`softcore portal`).
2. Softcore shows its picker: a card for each display and window with a small live picture,
   in the Rivet look. Arrows and Enter, or a click, choose; Esc says no. Softcore draws the
   picker itself, so no program can cover it, imitate it or answer it for you.
3. Softcore starts a PipeWire stream of what was chosen and the app reads it. A shared
   window keeps being shared when it scrolls out of view or is on another workspace, and
   the stream follows it when it is resized.
4. The stream ends when the app ends it, when the window closes, or when whoever asked goes
   away for any reason. Nothing is left sharing by accident.

The same without an app, which is also how to check it:

```sh
softcore msg pick-cast                         # shows the picker, prints the choice
softcore msg cast start output DP-5 --cursor   # prints the PipeWire node; shares until stopped (Ctrl+C)
softcore msg cast start window 4
softcore msg casts                             # what is being shared now
gst-launch-1.0 pipewiresrc path=NODE ! videoconvert ! autovideosink
```

Programs that capture the screen directly (`grim`, and screenshots from a shell) work too,
through ext-image-copy-capture.

For apps to reach it, three files from `data/` need installing (a package would do this):

| File | Where |
| --- | --- |
| `softcore.portal` | `/usr/share/xdg-desktop-portal/portals/` or `~/.local/share/xdg-desktop-portal/portals/` |
| `softcore-portals.conf` | `/usr/share/xdg-desktop-portal/` or `~/.config/xdg-desktop-portal/` |
| `softcore.desktop` | `/usr/share/wayland-sessions/`, for login managers |

On displays of its own Softcore runs `softcore portal` itself; in a window, `--portal` does.

Frames are copied through memory, which is simple and works everywhere, and costs more than
it should on a 4K display: handing the GPU's own buffers to PipeWire is the next step there.

## What clients get

xdg-shell with server-side decorations, layer-shell, session lock, the clipboard, primary
selection and data-control, drag and drop, idle notify and idle inhibit, xdg-activation,
foreign-toplevel-list, screen capture (ext-image-copy-capture, of displays and of windows),
virtual keyboards and pointers, blur behind a surface (ext-background-effect), fractional scale, viewporter,
cursor-shape, single-pixel buffers, and buffers straight from the GPU (linux-dmabuf).

## What isn't built yet

- **A run on real displays.** The TTY backend (DRM, libinput, libseat, TTY switching) is
  written from smithay's own examples and compiles; nobody has started it on hardware yet.
  Expect to fix things there first. It draws with one GPU.
- Real keyboards and mice. Bindings, typing and click-to-focus are checked through virtual
  devices (`wtype`, `wlrctl`), which take the same path; libinput devices, and with them
  touchpad settings, wait for the first run on hardware. The wheel and menus that close on
  a click outside are written but not checked.
- Screen sharing from a real app. Every part is checked on its own (the D-Bus service
  called by hand, the picker, the stream read by GStreamer), but not yet a browser going
  through xdg-desktop-portal, which needs a Softcore session and the files above installed.
  Sharing more than one thing in one request, and remembering a choice, aren't built.
- X11 apps (no Xwayland), input methods, pointer locks for games, touch and tablets, touchpad
  gestures, presentation timing.
- Opacity per program: it is one setting for every unfocused window (`inactive_opacity`)
  until there are window rules. Blur doesn't follow a surface's round corners or its
  see-through parts; it fills the rectangle.
- Animations for floating windows, which jump, as a floating window going to full screen does.
- Window rules, named workspaces. Titles on the tabs: the strip shows how many there are and
  which is showing, not what they are. In the overview, dragging a window to
  another workspace, a corner of the screen that opens it, and a touchpad gesture for it.
  A dialog opens in the
  middle of the screen, not over its parent, and a window in a column can't be dragged
  out of it with the pointer. Scratchpads with names of their own, and one that starts its program when
  first asked for, wait for window rules.
- Clipping a window's own corners: the border is drawn over them, which reads as rounded,
  but a window with no border has square corners.

## Checks

```sh
cargo test --workspace          # the layout's rules and slides, the settings file both ways, the bindings
softcore --headless --size 1600x900 --spawn rivet-terminal &
softcore msg screenshot /tmp/softcore.png     # with SOFTCORE_SOCKET from its first lines
wtype -M alt -k Return -m alt                 # keys, with WAYLAND_DISPLAY from its first lines
wlrctl pointer click left                     # and the pointer
```

## Licence

MIT or Apache 2.0, at your choice, like the Rivet apps.
