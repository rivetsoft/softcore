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
- **`rivet-xdg`**: where the settings live.

The crates are pinned to a commit in `Cargo.toml`. To build against a checkout of the apps
repository instead, copy `.cargo/config.toml.example` to `.cargo/config.toml` and set the path.

Rivet Shell runs under Softcore today (panels, dock, wallpaper, launcher: they are layer-shell
surfaces), but doesn't know Softcore by name yet, so its workspaces item and window look
settings do nothing here. `softcore msg` answers with the same fields `niri msg` does, so
teaching the shell is a matter of a new `Compositor::Softcore` that runs `softcore msg`.

## Settings

`~/.config/softcore/settings`, `key=value` lines like every Rivet app's. Unknown keys are
skipped, lines that can't be read are listed in a panel, and the file is read again when it
changes. `softcore validate` checks it without starting anything.

| Key | What it sets |
| --- | --- |
| `gaps`, `gaps_out` | Between windows, and between windows and the screen's edges |
| `border`, `radius` | The border's width, and its corner radius (empty: Rivet's) |
| `border_active`, `border_inactive`, `background` | Colours as `#rrggbb` (empty: Rivet's) |
| `inactive_opacity`, `shadows` | Unfocused windows' opacity (0.1–1), shadows on or off |
| `column_widths`, `default_width` | The widths Mod+R steps through, and a new column's, as parts of the screen |
| `theme`, `accent`, `follow_desktop_colors` | The Rivet look: `dark`, `light` or `system`; an accent; whether the desktop's colours win |
| `mod` | `super`, `alt`, or `auto` |
| `xkb_layout`, `xkb_variant`, `xkb_options`, `repeat_delay`, `repeat_rate` | The keyboard |
| `tap_to_click`, `natural_scroll` | Touchpads |
| `cursor_theme`, `cursor_size` | The pointer |
| `spawn=COMMAND` | Run at start (repeatable): `spawn=rivet-shell` |
| `env=NAME=VALUE` | For everything Softcore starts (repeatable) |
| `bind=KEYS ACTION` | A key binding (repeatable); replaces a default on the same keys |
| `unbind=KEYS`, `default_binds=false` | Drop one default, or all of them |
| `output=NAME …` | A display: `mode=2560x1440@59.951 scale=1.5 position=0,0`, or `off` |
| `include=FILE` | Read another file here; later lines win |

A binding is keys and an action: `bind=Mod+T spawn foot`. Put `allow-when-locked` before the
action for keys that should work on the lock screen. The actions are named as niri names them
(`focus-column-left`, `move-column-to-workspace 3`, `set-column-width +10%`, …);
`src/config.rs` has the list, with the default bindings.

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
foreign-toplevel-list, fractional scale, viewporter, cursor-shape, single-pixel buffers, and
buffers straight from the GPU (linux-dmabuf).

## What isn't built yet

- **A run on real displays.** The TTY backend (DRM, libinput, libseat, TTY switching) is
  written from smithay's own examples and compiles; nobody has started it on hardware yet.
  Expect to fix things there first. It draws with one GPU.
- **Keys and the pointer under test.** Bindings, clicks and the wheel are handled, but the
  checks so far drive Softcore through `softcore msg` (there is no input in memory), so that
  path has only been compiled, not exercised.
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
cargo test                      # the layout's rules and the settings reader
softcore --headless --size 1600x900 --spawn rivet-terminal &
softcore msg screenshot /tmp/softcore.png     # with SOFTCORE_SOCKET from its first lines
```

## Licence

MIT or Apache 2.0, at your choice, like the Rivet apps.
