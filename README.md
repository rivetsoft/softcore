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
  the desktop's colours (a new wallpaper recolours the borders within two seconds); the other
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
- **Softcore reads the file again when it changes**, within two seconds, whoever changed it.

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
`border_inactive`, `inactive_opacity`, `shadows`, `column_widths`, `default_width`, `theme`,
`accent`, `follow_desktop_colors`, `background`, `mod`, `default_binds`, `xkb_layout`,
`xkb_variant`, `xkb_options`, `repeat_delay`, `repeat_rate`, `tap_to_click`,
`natural_scroll`, `cursor_theme`, `cursor_size`. `softcore settings` says what each is.

A binding's value is an action, named as niri names them (`focus-column-left`,
`move-column-to-workspace 3`, `set-column-width +10%`, …), with `allow-when-locked` before it
for keys that should work on the lock screen. `Mod` is Super on displays of its own and Alt
in a window. The actions and the default bindings are in `crates/softcore-settings/src/lib.rs`.

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

## What clients get

xdg-shell with server-side decorations, layer-shell, session lock, the clipboard, primary
selection and data-control, drag and drop, idle notify and idle inhibit, xdg-activation,
foreign-toplevel-list, virtual keyboards and pointers, fractional scale, viewporter,
cursor-shape, single-pixel buffers, and buffers straight from the GPU (linux-dmabuf).

## What isn't built yet

- **A run on real displays.** The TTY backend (DRM, libinput, libseat, TTY switching) is
  written from smithay's own examples and compiles; nobody has started it on hardware yet.
  Expect to fix things there first. It draws with one GPU.
- Real keyboards and mice. Bindings, typing and click-to-focus are checked through virtual
  devices (`wtype`, `wlrctl`), which take the same path; libinput devices, and with them
  touchpad settings, wait for the first run on hardware. The wheel and menus that close on
  a click outside are written but not checked.
- Screen capture for other programs (screencopy, so screenshots and screen sharing from
  Rivet Shell or a portal), and screenshots of a real display. `softcore msg screenshot`
  works in a window and headless.
- X11 apps (no Xwayland), input methods, pointer locks for games, touch and tablets, touchpad
  gestures, presentation timing.
- Window rules, named workspaces, tabbed columns, moving and resizing floating windows with
  the pointer, an overview.
- Clipping a window's own corners: the border is drawn over them, which reads as rounded,
  but a window with no border has square corners.

## Checks

```sh
cargo test --workspace          # the layout's rules, the settings file both ways, the bindings
softcore --headless --size 1600x900 --spawn rivet-terminal &
softcore msg screenshot /tmp/softcore.png     # with SOFTCORE_SOCKET from its first lines
wtype -M alt -k Return -m alt                 # keys, with WAYLAND_DISPLAY from its first lines
wlrctl pointer click left                     # and the pointer
```

## Licence

MIT or Apache 2.0, at your choice, like the Rivet apps.
