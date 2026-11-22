//! Sharing a display or a window as video: a PipeWire stream that other programs (a browser
//! in a call, a recorder) read. Softcore draws each frame into memory and copies it into the
//! stream's buffers.
//!
//! A cast lives as long as the connection that asked for it (`softcore msg cast …`, or the
//! portal): when that closes, the cast stops, so nothing is ever left sharing by accident.

use std::cell::RefCell;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use pw::stream::{StreamFlags, StreamState};
use serde_json::json;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};

use crate::capture::{Source, capture};
use crate::state::State;

/// At most this many frames a second.
const FRAME_GAP: Duration = Duration::from_millis(32);

struct Pipewire {
    _main_loop: pw::main_loop::MainLoopRc,
    _context: pw::context::ContextRc,
    core: pw::core::CoreRc,
}

/// What the stream's callbacks tell the rest.
#[derive(Default)]
struct Shared {
    node_id: Option<u32>,
    streaming: bool,
    /// The size agreed with whoever is reading.
    size: Option<(u32, u32)>,
    failed: Option<String>,
}

pub struct Cast {
    pub id: u32,
    pub source: Source,
    pub cursor: bool,
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<()>,
    shared: Rc<RefCell<Shared>>,
    /// Who asked: told the stream's number once there is one, and watched for leaving.
    owner: Option<UnixStream>,
    told: bool,
    /// The size last offered.
    offered: (u32, u32),
    last_frame: Instant,
}

#[derive(Default)]
pub struct Casts {
    pipewire: Option<Pipewire>,
    pub list: Vec<Cast>,
    next: u32,
}

fn pod(value: spa::pod::Value) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value).map(|r| r.0.into_inner()).unwrap_or_default()
}

/// The video Softcore can give: four bytes a pixel, at the source's own size.
fn format_pod(size: (u32, u32)) -> Vec<u8> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat;
    let rect = spa::utils::Rectangle { width: size.0, height: size.1 };
    pod(spa::pod::Value::Object(spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(FormatProperties::VideoFormat, Choice, Enum, Id, VideoFormat::RGBx, VideoFormat::RGBx, VideoFormat::RGBA),
        spa::pod::property!(FormatProperties::VideoSize, Rectangle, rect),
        spa::pod::property!(FormatProperties::VideoFramerate, Fraction, spa::utils::Fraction { num: 0, denom: 1 }),
        spa::pod::property!(FormatProperties::VideoMaxFramerate, Choice, Range, Fraction, spa::utils::Fraction { num: 30, denom: 1 }, spa::utils::Fraction { num: 1, denom: 1 }, spa::utils::Fraction { num: 60, denom: 1 }),
    )))
}

/// The buffers frames are copied into: plain shared memory, one block, whole frames.
fn buffers_pod(size: (u32, u32)) -> Vec<u8> {
    use spa::pod::{Property, PropertyFlags, Value};
    let prop = |key: u32, value: Value| Property { key, flags: PropertyFlags::empty(), value };
    let stride = size.0 as i32 * 4;
    pod(Value::Object(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamBuffers.as_raw(),
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![
            prop(spa::sys::SPA_PARAM_BUFFERS_buffers, Value::Int(4)),
            prop(spa::sys::SPA_PARAM_BUFFERS_blocks, Value::Int(1)),
            prop(spa::sys::SPA_PARAM_BUFFERS_size, Value::Int(stride * size.1 as i32)),
            prop(spa::sys::SPA_PARAM_BUFFERS_stride, Value::Int(stride)),
            prop(spa::sys::SPA_PARAM_BUFFERS_dataType, Value::Int(1 << spa::sys::SPA_DATA_MemFd)),
        ],
    }))
}

impl Casts {
    /// Reach PipeWire, once, and have its events handled with everything else's.
    fn pipewire(&mut self, state_handle: &smithay::reexports::calloop::LoopHandle<'static, State>) -> Result<&Pipewire, String> {
        if self.pipewire.is_none() {
            pw::init();
            let err = |e: pw::Error| format!("couldn't reach PipeWire: {e}");
            let main_loop = pw::main_loop::MainLoopRc::new(None).map_err(err)?;
            let context = pw::context::ContextRc::new(&main_loop, None).map_err(err)?;
            let core = context.connect_rc(None).map_err(err)?;
            let fd = main_loop.loop_().fd().try_clone_to_owned().map_err(|e| format!("couldn't reach PipeWire: {e}"))?;
            let pump = main_loop.clone();
            state_handle
                .insert_source(Generic::new(fd, Interest::READ, Mode::Level), move |_, _, _| {
                    pump.loop_().iterate(pw::loop_::Timeout::Finite(Duration::ZERO));
                    Ok(PostAction::Continue)
                })
                .map_err(|e| format!("couldn't reach PipeWire: {e}"))?;
            self.pipewire = Some(Pipewire { _main_loop: main_loop, _context: context, core });
        }
        Ok(self.pipewire.as_ref().unwrap())
    }
}

/// Start sharing. The asker is told `{"ok": {"id", "node_id"}}` once PipeWire has given the
/// stream a number, and the cast ends when the asker's connection does.
pub fn start(state: &mut State, source: Source, cursor: bool, owner: UnixStream) -> Result<(), (String, UnixStream)> {
    let Some(size) = state.source_size(&source).map(|s| (s.w as u32, s.h as u32)) else {
        return Err(("there is no such display or window to share".into(), owner));
    };
    let handle = state.handle.clone();
    let core = match state.casts.pipewire(&handle) {
        Ok(p) => p.core.clone(),
        Err(e) => return Err((e, owner)),
    };
    let made = (|| -> Result<_, pw::Error> {
        let stream = pw::stream::StreamRc::new(core, "softcore-screen-cast", properties! { *pw::keys::MEDIA_CLASS => "Video/Source", *pw::keys::MEDIA_ROLE => "Screen", *pw::keys::NODE_NAME => "softcore-screen-cast" })?;
        let shared = Rc::new(RefCell::new(Shared::default()));
        let (on_state, on_param) = (shared.clone(), shared.clone());
        let listener = stream
            .add_local_listener::<()>()
            .state_changed(move |stream, _, _, new| {
                let mut s = on_state.borrow_mut();
                match new {
                    StreamState::Paused => {
                        s.node_id = Some(stream.node_id());
                        s.streaming = false;
                    }
                    StreamState::Streaming => s.streaming = true,
                    StreamState::Error(e) => s.failed = Some(e),
                    _ => s.streaming = false,
                }
            })
            .param_changed(move |stream, _, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let mut info = spa::param::video::VideoInfoRaw::default();
                if info.parse(param).is_err() {
                    return;
                }
                let size = (info.size().width, info.size().height);
                on_param.borrow_mut().size = Some(size);
                let bytes = buffers_pod(size);
                if let Some(p) = Pod::from_bytes(&bytes) {
                    let _ = stream.update_params(&mut [p]);
                }
            })
            .register()?;
        let bytes = format_pod(size);
        let mut params = [Pod::from_bytes(&bytes).expect("a format made here is a pod")];
        stream.connect(spa::utils::Direction::Output, None, StreamFlags::DRIVER | StreamFlags::MAP_BUFFERS, &mut params)?;
        Ok((stream, listener, shared))
    })();
    let (stream, listener, shared) = match made {
        Ok(m) => m,
        Err(e) => return Err((format!("couldn't start sharing: {e}"), owner)),
    };
    let _ = owner.set_nonblocking(true);
    state.casts.next += 1;
    let id = state.casts.next;
    state.casts.list.push(Cast { id, source, cursor, stream, _listener: listener, shared, owner: Some(owner), told: false, offered: size, last_frame: Instant::now() - FRAME_GAP });
    state.queue_redraw();
    Ok(())
}

pub fn stop(state: &mut State, id: u32) -> bool {
    let before = state.casts.list.len();
    state.casts.list.retain(|c| c.id != id);
    before != state.casts.list.len()
}

pub fn list(state: &State) -> serde_json::Value {
    state
        .casts
        .list
        .iter()
        .map(|c| {
            let s = c.shared.borrow();
            let (kind, target) = match &c.source {
                Source::Output(n) => ("output", json!(n)),
                Source::Window(id) => ("window", json!(id)),
            };
            json!({"id": c.id, "kind": kind, "target": target, "node_id": s.node_id, "is_active": s.streaming, "cursor": c.cursor})
        })
        .collect()
}

/// Once round the loop: tell askers their stream's number, and end casts whose asker has
/// gone, whose source has gone, or that PipeWire gave up on.
pub fn refresh(state: &mut State) {
    if state.casts.list.is_empty() {
        return;
    }
    let mut wake = false;
    let mut list = std::mem::take(&mut state.casts.list);
    list.retain_mut(|c| {
        let shared = c.shared.borrow();
        if let Some(e) = &shared.failed {
            tracing::warn!("sharing stopped: {e}");
            return false;
        }
        if state.source_size(&c.source).is_none() {
            return false;
        }
        if let Some(owner) = &mut c.owner {
            if !c.told
                && let Some(node) = shared.node_id
            {
                c.told = true;
                let _ = writeln!(owner, "{}", json!({"ok": {"id": c.id, "node_id": node, "width": c.offered.0, "height": c.offered.1}}));
            }
            // Anything read is ignored; the end of the connection is the end of the cast.
            let mut byte = [0u8; 16];
            match std::io::Read::read(owner, &mut byte) {
                Ok(0) => return false,
                Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => return false,
                _ => {}
            }
        }
        // Someone started reading: they need a first frame without waiting for a change.
        if shared.streaming && c.last_frame.elapsed() > Duration::from_secs(1) {
            wake = true;
        }
        true
    });
    state.casts.list = list;
    if wake {
        state.queue_redraw();
    }
}

/// After a frame is drawn: the next frame of each cast someone is reading.
pub fn after_draw(state: &mut State, renderer: &mut GlesRenderer) {
    if state.casts.list.is_empty() {
        return;
    }
    let mut list = std::mem::take(&mut state.casts.list);
    for c in &mut list {
        let (streaming, agreed) = {
            let s = c.shared.borrow();
            (s.streaming, s.size)
        };
        if !streaming || c.last_frame.elapsed() < FRAME_GAP {
            continue;
        }
        let Some(size) = state.source_size(&c.source).map(|s| (s.w as u32, s.h as u32)) else { continue };
        if agreed != Some(size) {
            // The window was resized (or the display's mode changed): offer the new size and
            // carry on when the reader has taken it.
            if c.offered != size {
                c.offered = size;
                let bytes = format_pod(size);
                if let Some(p) = Pod::from_bytes(&bytes) {
                    let _ = c.stream.update_params(&mut [p]);
                }
            }
            continue;
        }
        let Some(shot) = capture(state, renderer, &c.source, c.cursor, false) else { continue };
        let Some(mut buffer) = c.stream.dequeue_buffer() else { continue };
        let datas = buffer.datas_mut();
        let Some(data) = datas.first_mut() else { continue };
        let bytes = shot.rgba.len();
        let Some(out) = data.data() else { continue };
        if out.len() < bytes {
            continue;
        }
        out[..bytes].copy_from_slice(&shot.rgba);
        let chunk = data.chunk_mut();
        *chunk.offset_mut() = 0;
        *chunk.size_mut() = bytes as u32;
        *chunk.stride_mut() = shot.w * 4;
        drop(buffer);
        let _ = c.stream.trigger_process();
        c.last_frame = Instant::now();
    }
    state.casts.list = list;
}
