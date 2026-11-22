<img src="assets/icon.svg" width="64" height="64" alt="">

# Softcore

The scrolling-tiling Wayland compositor for the Rivet desktop, in Rust on
[smithay](https://github.com/Smithay/smithay). Windows sit in columns on a strip that runs off
both sides of the screen; each display has its own stack of workspaces, with an empty one
always last. It works the way [niri](https://github.com/YaLTeR/niri) does, and is written from
scratch: niri's behaviour was studied from the outside (what it advertises to clients, what
`niri msg` says), none of its code is here.

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
Mod+Shift+/ shows every key. The ones to know first: Mod+Return a terminal, Mod+Space the
launcher, Mod+Q close, Mod+arrows move the focus, Mod+Ctrl+arrows move the window, Mod+R the
next column width, Mod+F a column as wide as the screen, Mod+Page Up and Down the workspaces,
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

Rivet Shell runs under Softcore today (panels, dock, wallpaper, launcher: they are layer-shell
surfaces), but doesn't know Softcore by name yet, so its workspaces item and window look
settings do nothing here. `softcore msg` answers with the same fields `niri msg` does, so
teaching the shell is a matter of a new `Compositor::Softcore` that runs `softcore msg`.

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
`border_inactive`, `inactive_opacity`, `shadows`, `shadow_size`, `shadow_offset`,
`shadow_opacity`, `shadow_color`, `blur`, `blur_strength`, `blur_layers`, `animations`,
`animation_speed`, `column_widths`, `default_width`, `theme`,
`accent`, `follow_desktop_colors`, `background`, `mod`, `default_binds`, `xkb_layout`,
`xkb_variant`, `xkb_options`, `repeat_delay`, `repeat_rate`, `tap_to_click`,
`natural_scroll`, `cursor_theme`, `cursor_size`. `softcore settings` says what each is.

A binding's value is an action, named as niri names them (`focus-column-left`,
`move-column-to-workspace 3`, `set-column-width +10%`, …), with `allow-when-locked` before it
for keys that should work on the lock screen. `Mod` is Super on displays of its own and Alt
in a window. The actions and the default bindings are in `crates/softcore-settings/src/lib.rs`.

## Motion, shadows and blur

Windows fade in a little small and grow into place, and fade out the same way; a window
that changes column, width or height slides and grows to its new place, and its neighbours
slide to make room. The strip and the workspaces slide as before. `animation_speed` makes
all of it faster or slower (2 is twice as fast), and `animations=false` puts everything
where it belongs at once.

While a window is on its way, its picture is stretched to the size it has got to, so the
slide is smooth whenever the program itself gets round to drawing at the new size. A
program that is killed outright vanishes without fading: its last frame goes with it.

Shadows have a size (larger is softer), an offset below the window, a strength and a
colour: `shadow_size`, `shadow_offset`, `shadow_opacity`, `shadow_color`.

What shows through a window that isn't solid (a see-through terminal, an unfocused window
with `inactive_opacity` below 1) is blurred: `blur`, and `blur_strength` from 1 to 10.
Behind a window that is the wallpaper, which is all a tiled window has behind it, so its
blur is made once and kept until the wallpaper changes. A floating window shows the blurred
wallpaper too, not the windows under it.

Shell surfaces get blur behind them when they ask for it (ext-background-effect, with the
part of themselves they want it behind), or when `blur_layers` names them: by default
Rivet Shell's panel and dock. Behind those everything is blurred, windows included, and
made again only when something under them changed. A surface named in `blur_layers` is
blurred behind all of its rectangle, so name only ones that fill theirs.

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
- Animations for floating windows and for going fullscreen and back, which jump.
- Window rules, named workspaces, tabbed columns, moving and resizing floating windows with
  the pointer, an overview.
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
