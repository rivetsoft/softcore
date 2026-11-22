//! The pointer's pictures, from the desktop's cursor theme.

use std::collections::HashMap;
use std::io::Read;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::CursorIcon;
use smithay::utils::Transform;

pub struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    /// By name: the picture and its hot spot (None: the theme has no such cursor).
    loaded: HashMap<String, Option<(MemoryRenderBuffer, (i32, i32))>>,
}

impl Cursors {
    /// `theme` empty: the desktop's (XCURSOR_THEME), else "default".
    pub fn new(theme: &str, size: u32) -> Cursors {
        let name = if theme.is_empty() { std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into()) } else { theme.to_string() };
        Cursors { theme: xcursor::CursorTheme::load(&name), size, loaded: HashMap::new() }
    }

    fn load(&self, name: &str) -> Option<(MemoryRenderBuffer, (i32, i32))> {
        let path = self.theme.load_icon(name)?;
        let mut bytes = Vec::new();
        std::fs::File::open(path).ok()?.read_to_end(&mut bytes).ok()?;
        let images = xcursor::parser::parse_xcursor(&bytes)?;
        // The size nearest the one asked for; the first frame of an animated cursor.
        let img = images.iter().min_by_key(|i| (i.size as i64 - self.size as i64).abs())?;
        let buffer = MemoryRenderBuffer::from_slice(&img.pixels_rgba, Fourcc::Abgr8888, (img.width as i32, img.height as i32), 1, Transform::Normal, None);
        Some((buffer, (img.xhot as i32, img.yhot as i32)))
    }

    /// A cursor's picture and hot spot; the arrow where the theme lacks the one asked for.
    pub fn get(&mut self, icon: CursorIcon) -> Option<(MemoryRenderBuffer, (i32, i32))> {
        for name in [icon.name(), "default", "left_ptr"] {
            if !self.loaded.contains_key(name) {
                let found = self.load(name).or_else(|| icon.alt_names().iter().find_map(|n| self.load(n)).filter(|_| name == icon.name()));
                self.loaded.insert(name.to_string(), found);
            }
            if let Some(Some(c)) = self.loaded.get(name) {
                return Some(c.clone());
            }
        }
        // No theme at all: a small arrow drawn here, so the pointer is never invisible.
        if !self.loaded.contains_key("") {
            self.loaded.insert(String::new(), Some((arrow(), (0, 0))));
        }
        self.loaded.get("").cloned().flatten()
    }
}

fn arrow() -> MemoryRenderBuffer {
    const N: usize = 16;
    let mut px = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            // A triangle with a dark edge.
            let inside = x <= y && x + y / 2 < N - 2 && y < N - 2;
            let edge = inside && (x == 0 || x == y || x + y / 2 == N - 3 || y == N - 3);
            if inside {
                let v = if edge { 0 } else { 255 };
                px[(y * N + x) * 4..(y * N + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
    MemoryRenderBuffer::from_slice(&px, Fourcc::Abgr8888, (N as i32, N as i32), 1, Transform::Normal, None)
}
