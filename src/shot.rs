//! Screenshots. `screenshot` dims the screen and waits to be told what the picture is of: an
//! area dragged out with the pointer, a window clicked, or the whole display. Softcore draws
//! this itself, so it works over menus and full-screen windows, and nothing of it is in the
//! picture. `screenshot-screen` and `screenshot-window` take theirs at once.
//!
//! A picture is saved as a PNG and put on the clipboard, and a notification says where.
//!
//! An app can ask too, through the portal (`softcore portal`): `pick-screenshot FILE` shows
//! the same chooser and answers once the picture is in that file, and `pick-color` waits for a
//! click and answers with the color of the pixel there. Neither touches the clipboard.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::input::keyboard::{Keysym, ModifiersState};
use smithay::reexports::calloop::channel;
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};
use serde_json::json;
use smithay::wayland::selection::data_device::set_data_device_selection;

use crate::capture::{Shot, Source, capture};
use crate::layout::WinId;
use crate::render::Elem;
use crate::state::State;

/// Choosing what to take a picture of.
pub struct Shooter {
    /// The display the area is on.
    pub output: String,
    /// The area chosen, within its display.
    pub area: Option<Rectangle<i32, Logical>>,
    /// Where the button went down, while it is held.
    from: Option<Point<f64, Logical>>,
    dragged: bool,
    /// Whether the pointer is in the picture.
    pub pointer: bool,
    ids: [Id; 8],
    /// Who asked, when it was an app: told the file or the color, or that nothing was chosen.
    asker: Option<Asker>,
}

/// An app that asked for a picture (through the portal), and what it wants.
pub struct Asker {
    stream: UnixStream,
    /// Where the picture goes. None: the app wants a color, not a picture.
    file: Option<PathBuf>,
    _hangup: crate::state::Hangup,
}

/// What a picture is of.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Area(String, Rectangle<i32, Logical>),
    Screen(String),
    Window(WinId),
}

/// A picture to take when the next frame has been drawn.
pub struct Wanted {
    pub what: What,
    /// Saved here and nothing more, for scripts. None: the usual folder, clipboard and note.
    pub file: Option<PathBuf>,
    pub pointer: bool,
    pub save: bool,
    /// Opened afterwards in the program that marks pictures up (`screenshot_edit`).
    pub edit: bool,
    /// Told when the picture is in `file` (or the color, when `color`), for an app that asked.
    pub reply: Option<UnixStream>,
    /// Only the first pixel's color is wanted.
    pub color: bool,
}

/// What the note at the foot of the screen says.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// The area's size in pixels. None: the whole display.
    pub size: Option<(i32, i32)>,
    pub pointer: bool,
    /// Whether E would open the picture to be marked up.
    pub edit: bool,
    /// An app asked: the picture goes to it, so there is no saving or copying to choose.
    pub for_app: bool,
    /// A color is being picked, not a picture.
    pub color: bool,
}

/// The rectangle between two points, kept inside a display of this size.
pub fn between(a: Point<f64, Logical>, b: Point<f64, Logical>, size: (i32, i32)) -> Rectangle<i32, Logical> {
    let clamp = |v: f64, max: i32| (v.round() as i32).clamp(0, max);
    let (x0, x1) = (clamp(a.x.min(b.x), size.0), clamp(a.x.max(b.x), size.0));
    let (y0, y1) = (clamp(a.y.min(b.y), size.1), clamp(a.y.max(b.y), size.1));
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

/// The part of a picture inside a rectangle of it. None when nothing of it is inside.
pub fn crop(shot: &Shot, r: Rectangle<i32, Physical>) -> Option<Shot> {
    let (x0, y0) = (r.loc.x.clamp(0, shot.w), r.loc.y.clamp(0, shot.h));
    let (x1, y1) = ((r.loc.x + r.size.w).clamp(0, shot.w), (r.loc.y + r.size.h).clamp(0, shot.h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in y0 as usize..y1 as usize {
        let row = y * shot.w as usize * 4;
        rgba.extend_from_slice(&shot.rgba[row + x0 as usize * 4..row + x1 as usize * 4]);
    }
    Some(Shot { rgba, w: w as i32, h: h as i32 })
}

/// Where a picture taken now is saved: `Screenshot from 2026-11-23 10-22-33.png`, in the
/// folder the settings name (`screenshot_dir`).
pub fn default_file(screenshot_dir: &str) -> PathBuf {
    let home = rivet_xdg::places::home();
    let dir = match screenshot_dir.trim() {
        "" => home.join("Pictures/Screenshots"),
        d => match d.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(d),
        },
    };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    unsafe {
        libc::localtime_r(&now, &mut tm);
    }
    let name = format!("Screenshot from {:04}-{:02}-{:02} {:02}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec);
    // Two in one second: the second gets a number.
    let mut file = dir.join(format!("{name}.png"));
    let mut n = 2;
    while file.exists() {
        file = dir.join(format!("{name} ({n}).png"));
        n += 1;
    }
    file
}

/// Start choosing, or stop if already choosing.
pub fn open(state: &mut State) {
    if state.shooter.take().is_some() || state.lock.locked {
        state.queue_redraw();
        return;
    }
    let Some(output) = state.output_at(state.pointer_pos).or(state.active_output()).map(|o| o.name()) else { return };
    state.end_chord();
    state.shooter = Some(Shooter { output, area: None, from: None, dragged: false, pointer: false, ids: std::array::from_fn(|_| Id::new()), asker: None });
    state.queue_redraw();
}

/// An app asks for a picture into `file`, or for a color (`file` None). The person chooses as
/// with their own screenshots; the answer goes back on `stream`.
pub fn open_for(state: &mut State, file: Option<PathBuf>, mut stream: UnixStream) {
    if state.shooter.is_some() || state.lock.locked {
        let _ = writeln!(stream, "{}", json!({"error": "a screenshot is already being chosen"}));
        return;
    }
    let Some(output) = state.output_at(state.pointer_pos).or(state.active_output()).map(|o| o.name()) else {
        let _ = writeln!(stream, "{}", json!({"error": "there is no display"}));
        return;
    };
    state.end_chord();
    let _ = stream.set_nonblocking(true);
    let hangup = crate::state::Hangup::new(&state.handle, &stream);
    let asker = Asker { stream, file, _hangup: hangup };
    state.shooter = Some(Shooter { output, area: None, from: None, dragged: false, pointer: false, ids: std::array::from_fn(|_| Id::new()), asker: Some(asker) });
    state.queue_redraw();
}

/// An app takes a picture of the focused display at once, told when it is in `file`.
pub fn take_for(state: &mut State, file: PathBuf, mut stream: UnixStream) {
    let Some(output) = state.active_output().map(|o| o.name()) else {
        let _ = writeln!(stream, "{}", json!({"error": "there is no display"}));
        return;
    };
    state.screenshot = Some(Wanted { what: What::Screen(output), file: Some(file), pointer: false, save: true, edit: false, reply: Some(stream), color: false });
    state.queue_redraw();
}

pub fn close(state: &mut State) {
    if let Some(mut a) = state.shooter.take().and_then(|sh| sh.asker) {
        let _ = writeln!(a.stream, "{}", json!({"error": "nothing was chosen"}));
    }
    state.queue_redraw();
}

/// Once round the loop: an app that asked and then went away takes its question with it.
pub fn refresh(state: &mut State) {
    let Some(a) = state.shooter.as_mut().and_then(|sh| sh.asker.as_mut()) else { return };
    let gone = match std::io::Read::read(&mut a.stream, &mut [0u8; 16]) {
        Ok(0) => true,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
        Ok(_) => false,
    };
    if gone {
        close(state);
    }
}

/// Take the picture chosen, and stop choosing.
fn finish(state: &mut State, what: Option<What>, save: bool, edit: bool) {
    let Some(sh) = state.shooter.take() else { return };
    let what = what.unwrap_or(match sh.area {
        Some(r) => What::Area(sh.output, r),
        None => What::Screen(sh.output),
    });
    state.screenshot = Some(match sh.asker {
        Some(a) => {
            let _ = a.stream.set_nonblocking(false);
            Wanted { what, color: a.file.is_none(), file: a.file, pointer: sh.pointer, save: true, edit: false, reply: Some(a.stream) }
        }
        None => Wanted { what, file: None, pointer: sh.pointer, save, edit, reply: None, color: false },
    });
    state.queue_redraw();
}

/// Whether the chooser is picking a color for an app.
fn picking_color(state: &State) -> bool {
    state.shooter.as_ref().and_then(|sh| sh.asker.as_ref()).is_some_and(|a| a.file.is_none())
}

/// The color under the pointer is the one picked.
fn pick_here(state: &mut State) {
    let pos = state.pointer_pos;
    let Some(output) = state.output_at(pos).cloned() else { return };
    let at = pos - State::output_geometry(&output).loc.to_f64();
    let size = State::output_size(&output);
    let (x, y) = ((at.x.floor() as i32).clamp(0, size.w - 1), (at.y.floor() as i32).clamp(0, size.h - 1));
    finish(state, Some(What::Area(output.name(), Rectangle::new((x, y).into(), (1, 1).into()))), true, false);
}

/// A key pressed while choosing. Every key is Softcore's until the choice is made.
pub fn key(state: &mut State, mods: &ModifiersState, sym: Keysym) {
    let Some(on) = state.shooter.as_ref().map(|sh| sh.output.clone()) else { return };
    if picking_color(state) {
        match sym {
            Keysym::Escape => close(state),
            Keysym::Return | Keysym::KP_Enter | Keysym::space => pick_here(state),
            _ => {}
        }
        return;
    }
    let for_app = state.shooter.as_ref().is_some_and(|sh| sh.asker.is_some());
    match sym {
        Keysym::Escape => close(state),
        Keysym::Return | Keysym::KP_Enter | Keysym::space => finish(state, None, true, false),
        Keysym::c if mods.ctrl => finish(state, None, false, false),
        Keysym::s => {
            let output = state.output_at(state.pointer_pos).map(|o| o.name()).unwrap_or(on);
            finish(state, Some(What::Screen(output)), true, false);
        }
        Keysym::w => {
            if let Some(id) = state.window_under(state.pointer_pos).or(state.layout.active_window()) {
                finish(state, Some(What::Window(id)), true, false);
            }
        }
        Keysym::e if !for_app && installed(&state.config.screenshot_edit) => finish(state, None, true, true),
        Keysym::p => {
            if let Some(sh) = &mut state.shooter {
                sh.pointer = !sh.pointer;
            }
            state.queue_redraw();
        }
        _ => {}
    }
}

/// A button pressed or let go while choosing. The left one drags out an area, or, let go
/// where it went down, picks the window there; any other stops.
pub fn button(state: &mut State, code: u32, pressed: bool) {
    if code != 0x110 {
        if pressed {
            close(state);
        }
        return;
    }
    let pos = state.pointer_pos;
    if picking_color(state) {
        if !pressed {
            pick_here(state);
        }
        return;
    }
    if pressed {
        let Some(output) = state.output_at(pos).cloned() else { return };
        let at = pos - State::output_geometry(&output).loc.to_f64();
        if let Some(sh) = &mut state.shooter {
            sh.output = output.name();
            sh.from = Some(at);
            sh.dragged = false;
            sh.area = None;
        }
    } else {
        let Some(sh) = &mut state.shooter else { return };
        if sh.from.take().is_some() && !sh.dragged {
            let name = sh.output.clone();
            let rect = state.window_under(pos).and_then(|id| {
                let output = state.output_named(&name)?;
                let size = State::output_size(output);
                let t = state.tiles(output).into_iter().find(|t| t.id == id)?;
                Some(between((t.rect.x as f64, t.rect.y as f64).into(), ((t.rect.x + t.rect.w) as f64, (t.rect.y + t.rect.h) as f64).into(), (size.w, size.h)))
            });
            if let Some(sh) = &mut state.shooter {
                sh.area = rect.filter(|r| r.size.w > 0 && r.size.h > 0);
            }
        }
    }
    state.queue_redraw();
}

/// The pointer moved while choosing.
pub fn moved(state: &mut State) {
    let pos = state.pointer_pos;
    let Some(sh) = &state.shooter else { return };
    let (Some(from), Some(output)) = (sh.from, state.output_named(&sh.output)) else { return };
    let at = pos - State::output_geometry(output).loc.to_f64();
    let size = State::output_size(output);
    // A press that hardly moves is a click.
    let far = (at.x - from.x).abs() > 3.0 || (at.y - from.y).abs() > 3.0;
    if let Some(sh) = &mut state.shooter
        && (sh.dragged || far)
    {
        sh.dragged = true;
        let r = between(from, at, (size.w, size.h));
        sh.area = (r.size.w > 0 && r.size.h > 0).then_some(r);
    }
}

fn solid(id: &Id, x: i32, y: i32, w: i32, h: i32, rgba: [f32; 4], scale: Scale<f64>, out: &mut Vec<Elem>) {
    if w <= 0 || h <= 0 {
        return;
    }
    let geo: Rectangle<i32, Physical> = Rectangle::<i32, Logical>::new((x, y).into(), (w, h).into()).to_physical_precise_round(scale);
    out.push(Elem::Solid(SolidColorRenderElement::new(id.clone(), geo, CommitCounter::default(), rgba, Kind::Unspecified)));
}

/// What choosing looks like on a display: the note, the area's outline, and everything else
/// dimmed.
pub fn elements(state: &mut State, renderer: &mut GlesRenderer, output: &Output, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let Some(sh) = &state.shooter else { return };
    let size = State::output_size(output);
    let for_app = sh.asker.is_some();
    let color = picking_color(state);
    let here = sh.output == output.name() || (color && state.output_at(state.pointer_pos).is_some_and(|o| o == output));
    let area = sh.area.filter(|_| here);
    let ids = sh.ids.clone();
    if here {
        let px = area.map(|r| {
            let p: Rectangle<i32, Physical> = r.to_physical_precise_round(scale);
            (p.size.w, p.size.h)
        });
        let px = px.filter(|_| !color);
        let view = View { size: px, pointer: sh.pointer, edit: !for_app && installed(&state.config.screenshot_edit), for_app, color };
        let (buffer, (w, h)) = state.ui.shot_buffer(&state.config, (size.w, size.h), &view);
        let loc = Point::<i32, Logical>::from(((size.w - w) / 2, (size.h - h - 48).max(0))).to_f64().to_physical(scale);
        if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, None, None, Kind::Unspecified) {
            out.push(Elem::Memory(e));
        }
    }
    // A color is picked from the screen as it is: nothing dimmed.
    if color {
        return;
    }
    let Some(r) = area else {
        solid(&ids[4], 0, 0, size.w, size.h, [0.0, 0.0, 0.0, 0.25], scale, out);
        return;
    };
    let c = state.ui.look(&state.config).active;
    let line = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0];
    let (x, y, w, h) = (r.loc.x, r.loc.y, r.size.w, r.size.h);
    // The outline goes round the outside, so none of it is over what will be in the picture.
    solid(&ids[0], x - 2, y - 2, w + 4, 2, line, scale, out);
    solid(&ids[1], x - 2, y + h, w + 4, 2, line, scale, out);
    solid(&ids[2], x - 2, y, 2, h, line, scale, out);
    solid(&ids[3], x + w, y, 2, h, line, scale, out);
    let dim = [0.0, 0.0, 0.0, 0.5];
    solid(&ids[4], 0, 0, size.w, y, dim, scale, out);
    solid(&ids[5], 0, y + h, size.w, size.h - y - h, dim, scale, out);
    solid(&ids[6], 0, y, x, h, dim, scale, out);
    solid(&ids[7], x + w, y, size.w - x - w, h, dim, scale, out);
}

fn png(shot: Shot) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(shot.w as u32, shot.h as u32, shot.rgba).ok_or("the frame was an unexpected size")?;
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).map_err(|e| format!("couldn't make the picture: {e}"))?;
    Ok(bytes)
}

/// Written beside under another name first, so the file is never there half written.
fn write(file: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut part = file.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    std::fs::File::create(&part).and_then(|mut f| f.write_all(bytes)).and_then(|()| std::fs::rename(&part, file)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("couldn't write {}: {e}", file.display())
    })
}

/// For the shell: single quotes round a word, whatever is in it.
fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Whether a command's program can be found: a path that is there, or a name on PATH.
fn installed(command: &str) -> bool {
    let Some(program) = command.split_whitespace().next() else { return false };
    if program.contains('/') {
        return std::path::Path::new(program).exists();
    }
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// Take a picture, now that a frame without the dimming has been drawn.
pub fn take(state: &mut State, renderer: &mut GlesRenderer, wanted: Wanted) {
    let source = match &wanted.what {
        What::Area(o, _) | What::Screen(o) => Source::Output(o.clone()),
        What::Window(id) => Source::Window(*id),
    };
    let Some(shot) = capture(state, renderer, &source, wanted.pointer, true) else {
        tracing::warn!("couldn't take a screenshot");
        if let Some(mut r) = wanted.reply {
            let _ = writeln!(r, "{}", json!({"error": "couldn't take a screenshot"}));
        }
        return;
    };
    let area = match &wanted.what {
        What::Area(o, r) => {
            let scale = state.output_named(o).map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0);
            Some(r.to_physical_precise_round(scale))
        }
        _ => None,
    };
    let job = Job { shot, area, file: wanted.file, save: wanted.save, edit: wanted.edit, dir: state.config.screenshot_dir.clone(), reply: wanted.reply, color: wanted.color };
    let Some(jobs) = worker(state) else {
        if let Some(m) = make(job) {
            made(state, m);
        }
        return;
    };
    // A worker that has gone (it panicked) is replaced next time; this picture is made here.
    if let Err(std::sync::mpsc::SendError(job)) = jobs.send(job) {
        state.shots.jobs = None;
        if let Some(m) = make(job) {
            made(state, m);
        }
    }
}

/// Pictures being cut, made into PNGs and saved one after another, away from the loop:
/// a PNG of a whole display takes 20 to 300 ms, and the desktop would stand still for it.
#[derive(Default)]
pub struct Shots {
    jobs: Option<std::sync::mpsc::Sender<Job>>,
}

struct Job {
    shot: Shot,
    /// The part of it wanted, in pixels. None: all of it.
    area: Option<Rectangle<i32, Physical>>,
    file: Option<PathBuf>,
    save: bool,
    edit: bool,
    /// The settings' `screenshot_dir`.
    dir: String,
    reply: Option<UnixStream>,
    color: bool,
}

/// A picture made, back on the loop for the clipboard, and the note or the editor.
struct Made {
    bytes: Vec<u8>,
    saved: Option<PathBuf>,
    save: bool,
    edit: bool,
}

/// The worker that makes pictures, started the first time one is taken.
fn worker(state: &mut State) -> Option<std::sync::mpsc::Sender<Job>> {
    if state.shots.jobs.is_none() {
        let (done, back) = channel::channel::<Made>();
        let (jobs, todo) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("screenshots".into())
            .spawn(move || {
                for job in todo {
                    if let Some(m) = make(job)
                        && done.send(m).is_err()
                    {
                        return;
                    }
                }
            })
            .map_err(|e| tracing::warn!("couldn't start making screenshots aside: {e}"))
            .ok()?;
        state
            .handle
            .insert_source(back, |event, _, state| {
                if let channel::Event::Msg(m) = event {
                    made(state, m);
                }
            })
            .map_err(|e| tracing::warn!("couldn't hear back from making screenshots: {e}"))
            .ok()?;
        state.shots.jobs = Some(jobs);
    }
    state.shots.jobs.clone()
}

/// Cut, encode and save a picture. None when there is nothing more to do on the loop.
fn make(mut job: Job) -> Option<Made> {
    let shot = match job.area {
        Some(r) => crop(&job.shot, r),
        None => Some(job.shot),
    };
    // For an app: the answer goes straight back from here, and nothing more happens.
    if let Some(mut reply) = job.reply.take() {
        let answer = match (shot, job.color, &job.file) {
            (None, ..) => json!({"error": "couldn't take a screenshot"}),
            (Some(s), true, _) => json!({"ok": [s.rgba[0], s.rgba[1], s.rgba[2]]}),
            (Some(mut s), false, Some(file)) => {
                s.make_opaque();
                match png(s).and_then(|bytes| write(file, &bytes)) {
                    Ok(()) => json!({"ok": file}),
                    Err(e) => json!({"error": e}),
                }
            }
            (Some(_), false, None) => json!({"error": "nowhere to put the picture"}),
        };
        let _ = writeln!(reply, "{answer}");
        return None;
    }
    let bytes = shot
        .ok_or_else(|| "couldn't take a screenshot".to_string())
        .and_then(|mut s| {
            s.make_opaque();
            png(s)
        })
        .map_err(|e| tracing::warn!("{e}"))
        .ok()?;
    if let Some(file) = &job.file {
        if let Err(e) = write(file, &bytes) {
            tracing::warn!("{e}");
        }
        return None;
    }
    let saved = job.save.then(|| default_file(&job.dir)).and_then(|file| match write(&file, &bytes) {
        Ok(()) => Some(file),
        Err(e) => {
            tracing::warn!("{e}");
            None
        }
    });
    Some(Made { bytes, saved, save: job.save, edit: job.edit })
}

/// The picture is made: onto the clipboard, then marked up or noted.
fn made(state: &mut State, m: Made) {
    let seat = state.seat.clone();
    set_data_device_selection(&state.dh, &seat, vec!["image/png".into()], Arc::from(m.bytes));
    // Marked up next: the program's own window says where the picture is. Where the program
    // isn't there, the picture is saved and copied all the same, and the note says so.
    let editor = state.config.screenshot_edit.trim().to_string();
    if let (true, Some(file)) = (m.edit, &m.saved)
        && installed(&editor)
    {
        state.spawn(&format!("{editor} {}", quoted(&file.display().to_string())));
        return;
    }
    let note = match &m.saved {
        Some(file) => format!("notify-send -a Softcore -i {} 'Screenshot saved' {}", quoted(&file.display().to_string()), quoted(&format!("{}, and on the clipboard", file.display()))),
        None if m.save => "notify-send -a Softcore 'Screenshot copied' 'It couldn'\\''t be saved, but it is on the clipboard'".to_string(),
        None => "notify-send -a Softcore 'Screenshot copied' 'On the clipboard'".to_string(),
    };
    state.spawn(&note);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_is_between_two_points_whichever_came_first_and_stays_on_the_display() {
        let r = between((300.0, 200.0).into(), (100.0, 50.0).into(), (1280, 800));
        assert_eq!((r.loc.x, r.loc.y, r.size.w, r.size.h), (100, 50, 200, 150));
        let r = between((1200.0, 700.0).into(), (1500.0, -40.0).into(), (1280, 800));
        assert_eq!((r.loc.x, r.loc.y, r.size.w, r.size.h), (1200, 0, 80, 700));
    }

    #[test]
    fn a_picture_is_cut_to_the_area() {
        // 4 by 3, each pixel its own number.
        let shot = Shot { rgba: (0..12u8).flat_map(|n| [n, 0, 0, 255]).collect(), w: 4, h: 3 };
        let c = crop(&shot, Rectangle::new((1, 1).into(), (2, 2).into())).unwrap();
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.rgba.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), vec![5, 6, 9, 10]);
        // Partly off the edge: what is on it. Wholly off: nothing.
        let c = crop(&shot, Rectangle::new((3, 2).into(), (5, 5).into())).unwrap();
        assert_eq!((c.w, c.h, c.rgba[0]), (1, 1, 11));
        assert!(crop(&shot, Rectangle::new((4, 0).into(), (2, 2).into())).is_none());
    }
}
