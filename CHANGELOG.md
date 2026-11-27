# Changelog

## 0.1.0

The first release, an early one: Softcore has run on its author's computer and nowhere else
yet, so keep another session to log in to.

- **Softcore, a scrolling Wayland compositor for the Rivet desktop.** Windows sit in columns
  on a strip that runs off both sides of the screen, and you scroll through them. Opening a
  window never shrinks the others.
- **Columns of any width**, holding one window or several. Mod+R steps through the widths,
  Mod+F makes a column as wide as the screen, and a window can be made shorter or taller in its
  column.
- **Workspaces per display**, with an empty one always last. Unplug a display and its
  workspaces move to another, then go back when it returns.
- **Up and down too**: a workspace can scroll vertically, which suits a tall display.
- **Tabs, floating windows and scratchpads.** A column can show its windows as tabs, any
  window can float, and named scratchpads come and go on a key. A floating window goes to
  another display on the same keys as a column, or by dragging it there.
- **The overview** shows every workspace of each display at once. Click a window or a
  workspace to go there.
- **Bindings for anything**, in one settings file of plain `key=value` lines that Softcore
  reads again when you save it. If you know niri's actions and `niri msg`, you know
  Softcore's.
- **Chords**: press one key, then another. A panel shows what can come next, and H, J, K and L
  work in it. Mod+Shift+/ shows every binding.
- **Rules** tell a window which workspace to open on, whether to float, and how see-through to
  be.
- **Screen sharing is built in.** The picker shows a card with a live preview for each display
  and window, and a shared window stays shared even when it is scrolled off screen.
- **Screenshots**: Print or Mod+S, then drag out an area, click a window or take the whole
  screen. The picture is saved and copied, and E opens it in Rivet Snip.
- **Dressed like the Rivet desktop**: borders and Softcore's own panels use the desktop's
  colors and change when the wallpaper does. Rivet Shell knows its workspaces, windows and
  displays, and any panel or dock made for layer-shell compositors works too.
- **Also**: blur behind see-through windows and panels, shadows, animations, touchpad
  gestures, X11 programs through Xwayland, a lock screen, and `softcore msg` with an event
  stream for scripts.
- **Install for everyone** from the release download or from source with `install.sh`, which
  also adds Softcore to the login screen's list of sessions. `./install.sh --uninstall` takes
  it away again.
