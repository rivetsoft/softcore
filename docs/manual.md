# Softcore: the manual

Everything Softcore does, in one page. The short version is the [README](../README.md).

Softcore works the way [niri](https://github.com/YaLTeR/niri) does, and is written from
scratch: niri's behaviour was studied from the outside (what it advertises to clients, what
`niri msg` says), none of its code is here. Windows sit in columns on a strip that runs off
both sides of the screen; each display has its own stack of workspaces, with an empty one
always last. When a display is unplugged its workspaces move to another display, and go back
to it when it returns. With no display left they wait for the next one.

Softcore is early. It runs on displays of its own, in a window of another desktop and with no
screen at all. The last two are checked by the tests and by hand; on real hardware it has
only been used on its author's machine (see *What isn't built yet*).

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
launcher, Mod+Q close, Mod+arrows move the focus, Mod+Shift+arrows move the window (and with Ctrl
as well, left and right take it to the next display), Mod+Ctrl+Left and Right go to the
next display, Mod+R the
next column width, Mod+Shift+Minus and Mod+Shift+Equal a shorter or taller window (it sits in
the middle of the height; Mod+Ctrl+R gives it all the height back), Mod+F a column as wide as the screen, Mod+Page Up and Down the workspaces, Mod+Shift+` puts a
window away in the scratchpad and Mod+` shows it floating over whatever is in view (and puts
it away again),
Mod+Shift+E leave.

## Installing

```sh
./install.sh               # builds Softcore, then asks for your password to install it
./install.sh --uninstall   # takes it all away again
```

That installs Softcore for everyone on the machine, and it is in the login manager's list
of sessions from the next time you log out:

| What | Where |
| --- | --- |
| The program | `/usr/local/bin/softcore` |
| The entry login managers list | `/usr/share/wayland-sessions/softcore.desktop` |
| What makes the session one to systemd | `/usr/lib/systemd/user/softcore-session.target` |
| The portal's files | `/usr/share/xdg-desktop-portal/portals/softcore.portal`, `/usr/share/xdg-desktop-portal/softcore-portals.conf` |

The script is `cargo build --release` and then `sudo softcore install --system`, which a
built Softcore can do on its own. For a package, `DESTDIR=/some/root softcore install
--system` writes the same files under that folder.

Without root, `softcore install` puts the session target and the portal's files under your
home (`~/.config`, `~/.local/share`). That is enough for the file picker and screen sharing
when you start Softcore from a TTY yourself, but a login manager won't list it.
`softcore uninstall`, with `--system` or without, takes away what the matching install
wrote.

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
  `env.NAME`, `rule.NAME`, `layer.NAME`, `workspace.NAME`, `scratchpad.NAME`,
  `chord.KEYS`), so each is one line that can be found and changed. An empty binding
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
`accent`, `follow_desktop_colors`, `background`, `mod`, `default_binds`, `chord_panel`,
`chord_vim`, `xwayland`, `xkb_layout`,
`xkb_variant`, `xkb_options`, `repeat_delay`, `repeat_rate`, `tap_to_click`,
`natural_scroll`, `cursor_theme`, `cursor_size`, `screenshot_dir`, `screenshot_edit`. `softcore settings` says what each is.

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

### Chords

A binding can be several keys pressed one after another, with commas between them:

```
bind.Mod+G,T=spawn rivet-terminal
bind.Mod+G,W,F=title="Browser" spawn firefox
bind.Mod+G,W,M=spawn thunderbird
chord.Mod+G=Go to
chord.Mod+G,W=Internet
```

The first keys open the chord, and a list in the Rivet look shows what can come next: each
key with what it does, and a `›` on those that lead on to more keys. Take the next step by
pressing its key, or go up and down the list with the arrow keys and press Enter. Backspace
goes back a step, Escape leaves, and so does any key that isn't part of the chord. Mod can
stay held for the keys after the first.

- `chord.KEYS=Name` names the list those keys open; without one it shows the keys.
- `title="…"` on a binding is what its line says, and `title=""` keeps it off the list while
  it still works.
- `chord_panel=false` shows nothing: the keys work the same, and the arrow keys are no
  longer taken.
- `chord_vim=true` adds J and K for down and up, L to take and H to go back. A step of the
  chord on one of those letters still wins.
- Keys that begin a chord are no longer a binding of their own: `bind.Mod+W,F=…` takes
  Mod+W away from what it did.

## Rules

Lines that say how particular things are treated. What follows the `=` is words with spaces
between; a word that holds a space goes in quotes, and `*` in a program's name or a title
stands for anything. Capitals don't matter.

### Windows

```
rule.video=app-id=mpv floating opacity=1
rule.chat=app-id=*discord* workspace=chat focus=false
rule.notes=title="* - Notes" width=30% border=off shadow=off
```

`app-id=` and `title=` say which windows the line is about (both, when both are given; every
window, when neither is). The rest is what those windows get:

| Word | What it does |
| --- | --- |
| `floating`, `tiled` | Opens floating over the columns, or in a column whatever it asked for. |
| `workspace=NAME` or `=N` | Opens on that workspace. |
| `output=NAME` | Opens on that display. |
| `width=40%` | The width of its column. |
| `maximized`, `fullscreen` | Opens as wide as the screen, or with the whole screen. |
| `focus=false` | Opens without taking the focus. |
| `scratchpad=NAME` | Opens put away in that scratchpad. |
| `opacity=0.9` | How solid it is, with the focus or without. Without this, `inactive_opacity` applies. |
| `border=off`, `shadow=off`, `blur=off` | Drawn without one. |

When several lines fit a window, each word is taken from the last line that has it. The top
group is used once, as the window opens; the last three rows follow the window, so a title
that changes can change how it looks. The name after `rule.` is only there so the line can
be found and changed.

### The shell's surfaces

Panels, wallpapers and launchers go by the name each gives itself, which `softcore msg
layers` lists:

```
layer.rivet-panel=opacity=0.9 blur shadow
layer.notif*=blur=off
```

`opacity`, `blur` (on, or off even where the surface asked for it) and `shadow`. `blur_layers`
still works and means the same as `blur` here.

### Workspaces with names

```
workspace.chat=output=DP-2 scroll=vertical
workspace.mail=
```

A named workspace is always there, empty or not, first in its display's stack in the order
the lines stand. `output=` says which display it is made on (and goes back to when that
display returns); `scroll=` which way it scrolls. A display with nothing on it yet starts on
its first named workspace.

Anywhere a workspace is given by number it can be given by name: `focus-workspace chat`,
`move-column-to-workspace mail`, `workspace=chat` in a rule. A name is found on whichever
display it is, and going to it takes the focus there. `softcore msg workspaces` and
ext-workspace give the name.

### Scratchpads with names

```
scratchpad.term=app-id=scratch-term command="rivet-terminal --class scratch-term" size=0.6
bind.Mod+N=toggle-scratchpad term
bind.Mod+Shift+N=move-window-to-scratchpad term
```

Each named scratchpad has its own key and its own windows; the scratchpad without a name
(``Mod+` ``) is separate from them all. `app-id=` and `title=` say which windows go into it as
they open. `command=` is run when it is asked for and holds nothing, and the window is shown
as soon as it opens, so one key starts the program the first time and shows and hides it
after that. `size=` is how much of the screen its window takes, in place of
`scratchpad_size`.

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

## Moving a window with the pointer

Mod and the left button picks a window up out of its column (a window's own title bar does
too, when it has one). It is drawn small by the pointer, and a mark in the accent colour
shows where it would go if let go of: a bar between two columns for a column of its own
there, or the upper or lower half of a window for a place above or below it in that
window's column (which adds a tab to a tabbed one). The outer quarters of a column are for
the bar, the middle for a place in it. On another display it goes to the workspace in view
there. Let go where it started, or before it has gone anywhere, and nothing moves.

Mod and the right button makes a column wider or narrower as the pointer goes left and
right. The edge nearer to where it was pressed is the one that follows the pointer: pressed
on the left half of the window, the left edge moves and the right one (and every column
after it) stays where it is, as far as the strip can be scrolled to allow that. Where
windows scroll up and down it is the row's height, the pointer goes up and down, and the
edges are the top and the bottom. On a floating window it resizes the window from the
nearest corner.

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
- The keys that move and size columns do the same to it: Mod+Shift+arrows move it,
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

## Screenshots

Print, or Mod+S (`screenshot`), dims the screen and waits to be told what the picture is of.
Drag out an area with the pointer, or click a window for an area that is exactly that
window, then press Enter. With nothing chosen, Enter takes the whole display.

A row at the foot of the screen names the keys that finish it (save, copy, edit where the
program for that is installed, cancel) and gives the size of the area chosen.

| Key | What it does |
| --- | --- |
| Enter, Space | Saves the picture and puts it on the clipboard. |
| Ctrl+C | Puts it on the clipboard without saving. |
| E | Saves it and opens it in Rivet Snip, to clip and mark up. |
| S | The whole display the pointer is on, at once. |
| W | The window under the pointer (or the focused one), at once, without its border. |
| P | Whether the pointer is in the picture. It is left out unless asked for. |
| Escape, another button | Stops without a picture. |

Softcore draws this itself, so it works over an open menu or a full-screen window, the
windows hear nothing from the pointer or the keys meanwhile, and none of it is in the
picture. Ctrl+Print (`screenshot-screen`) and Alt+Print (`screenshot-window`) take the
display and the focused window without asking.

A picture is saved as `Screenshot from 2026-11-23 10-22-33.png` in `Pictures/Screenshots`,
or in the folder `screenshot_dir` names, and a notification says where (through
`notify-send`). `screenshot-screen FILE` and `softcore msg screenshot FILE` save to that
file and do nothing else, for scripts. The PNG is made away from the desktop, which never
waits for it, so the file appears a moment after the command returns; it is written under
another name first, so a script that waits for it never reads half a picture.

`screenshot_edit` is the program E opens the picture in, with the file added to the end of
it: `rivet-snip --replace` unless you say otherwise (`swappy -f`, say), and empty for none.
What is saved there replaces the screenshot.
Where that program isn't installed, E does nothing.

## Talking to it

```sh
softcore msg windows            # also: outputs, workspaces, layers, focused-window, focused-output
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

For apps to reach it, and the desktop's file picker, a few files from `data/` have to be
in place; [Installing](#installing) puts them there.

The target is what makes the session one to systemd. On displays of its own Softcore tells
D-Bus and systemd where it is and starts `softcore-session.target`, which brings up
`graphical-session.target`: xdg-desktop-portal won't start without that, and nor will
anything else that waits for a graphical session. Softcore stops the target as it leaves.
Without the files Softcore still runs, and apps that ask the portal for a file picker or
the screen get no answer.

On displays of its own Softcore runs `softcore portal` itself; in a window, `--portal` does.

A stream gets a frame when what it shows changes, at most 30 a second, and one a second
when nothing does. Frames are copied from the GPU into the stream's memory once, which is
simple and works everywhere, and still costs more than it should on a 4K display: handing
the GPU's own buffers to PipeWire is the next step there.

## X11 programs

Programs written for X11 open through
[xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite), which has to be
installed (0.7 or later) along with Xwayland. Softcore takes an X11 display number when it
starts and gives it to what it runs as `DISPLAY`; satellite itself is started when the first
X11 program asks, and again if it ever stops. Each X11 window then arrives as an ordinary
window, so tiling, tabs, full screen and the rest treat it like any other.

`xwayland=false` turns it off; without satellite installed Softcore says so in its log and
carries on. Either is read when Softcore starts, not while it runs.

X11 programs only know about each other: one of them can't read the keys typed into a
Wayland window or take a picture of the screen the X11 way, and its main windows go where
Softcore puts them, not where it asks.

## Touchpads, touchscreens, pens and input methods

- **Three fingers on a touchpad** move what is in view: along the strip the windows scroll
  with them, and across it the workspaces do. Letting go leaves the nearest column, or
  workspace, in place. **Four fingers** up open the overview, and down close it. Swipes of
  two fingers, pinches and holds go to the program under the pointer.
- **A touchscreen** works as the program expects, and a finger put down gives the keyboard
  to what it lands on.
- **A drawing tablet's pen** moves the pointer and reaches the program as a pen, with its
  pressure, tilt and buttons.
- **A program may lock the pointer** in place and read how the mouse moves (games), or keep
  it inside its window.
- **Input methods** (fcitx5, ibus) can take what is typed and give text back, and their list
  of candidates is drawn by the text.

Of these only the pointer lock and fence can be checked without the hardware, and were; the
rest is written against the layout's own tests and waits for a run on a real machine (see
*What isn't built yet*). A touchscreen or tablet is taken to belong to the laptop's own
display, or the first one.

## What clients get

xdg-shell with server-side decorations, layer-shell, session lock, the clipboard, primary
selection and data-control, drag and drop, idle notify and idle inhibit, xdg-activation,
foreign-toplevel-list, ext-workspace (see below), screen capture (ext-image-copy-capture, of displays and of windows),
virtual keyboards and pointers, pointer constraints and relative pointer, pointer gestures,
tablet, text-input and input-method, blur behind a surface (ext-background-effect), fractional scale, viewporter,
cursor-shape, single-pixel buffers, and buffers straight from the GPU (linux-dmabuf).

A bar or dock that isn't Rivet's can list and switch workspaces through ext-workspace-v1.
Each display is a group, and its workspaces are named by their place in the stack (1, 2, …),
the empty last one included. A client can ask for a workspace to be shown; making, removing
and moving workspaces aren't offered, as Softcore makes and drops them itself.

## What isn't built yet

- **Other people's hardware.** The TTY backend (DRM, libinput, libseat, TTY switching) is
  used on one machine, with one GPU, a keyboard and a mouse. Nothing else has been
  tried: more than one GPU, touchpad settings and laptops' own keys are written and wait
  for someone who has them.
- Screen sharing from a real app. Every part is checked on its own (the D-Bus service
  called by hand, the picker, the stream read by GStreamer), but not yet a browser going
  through xdg-desktop-portal, which needs a Softcore session and the files above installed.
  Sharing more than one thing in one request, and remembering a choice, aren't built.
- Fingers, pens and input methods on real hardware: three- and four-finger swipes, touch,
  tablets and input methods are written and none has met a real device yet. A touchscreen
  or tablet can't be given a display of its own choosing. Presentation timing.
- Blur doesn't follow a surface's round corners or its
  see-through parts; it fills the rectangle.
- Animations for floating windows, which jump, as a floating window going to full screen does.
- In a rule, patterns are `*` only (no regular expressions), and there is nothing yet for
  keeping a window out of screen sharing. Titles on the tabs: the strip shows how many there are and
  which is showing, not what they are. In the overview, dragging a window to
  another workspace, a corner of the screen that opens it, and a touchpad gesture for it.
  A dialog opens in the
  middle of the screen, not over its parent. A window carried by the pointer can't be
  taken to a workspace out of view, or let go of as a floating window.
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

[MIT](../LICENSE-MIT) or [Apache-2.0](../LICENSE-APACHE), at your option, like the Rivet apps.
