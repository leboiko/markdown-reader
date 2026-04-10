use std::collections::HashMap;

use image::{DynamicImage, RgbaImage};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use resvg::usvg;

use crate::markdown::MermaidBlockId;

/// The state of a mermaid diagram in the render cache.
pub enum MermaidEntry {
    /// Background task has been spawned; image is not yet available.
    Pending,
    /// Image is ready and encoded for the detected graphics protocol.
    ///
    /// Boxed because `StatefulProtocol` is large (>256 bytes), and clippy
    /// warns about large enum variants that inflate every instance of the enum.
    Ready(Box<StatefulProtocol>),
    /// Rendering failed; display the source with this short error message.
    Failed(String),
    /// Graphics are disabled (e.g. inside tmux); display the source with a hint.
    SourceOnly(String),
}

/// Per-app cache mapping diagram ids to their render state.
pub struct MermaidCache {
    entries: HashMap<MermaidBlockId, MermaidEntry>,
}

impl MermaidCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get_mut(&mut self, id: &MermaidBlockId) -> Option<&mut MermaidEntry> {
        self.entries.get_mut(id)
    }

    /// Insert a new entry, overwriting any existing one.
    pub fn insert(&mut self, id: MermaidBlockId, entry: MermaidEntry) {
        self.entries.insert(id, entry);
    }

    /// Ensure `id` has an entry. If it already has one, do nothing and return
    /// `false`. If not, insert `Pending`, spawn a background render task, and
    /// return `true`.
    ///
    /// When `picker` is `None` (graphics disabled), inserts `SourceOnly`
    /// immediately and returns `false` — no task is spawned.
    pub fn ensure_queued(
        &mut self,
        id: MermaidBlockId,
        source: &str,
        picker: Option<&Picker>,
        action_tx: &tokio::sync::mpsc::UnboundedSender<crate::action::Action>,
        in_tmux: bool,
    ) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }

        let Some(picker) = picker else {
            let reason = if in_tmux {
                TMUX_DISABLED_REASON.to_string()
            } else {
                "graphics unavailable".to_string()
            };
            self.entries.insert(id, MermaidEntry::SourceOnly(reason));
            return false;
        };

        self.entries.insert(id, MermaidEntry::Pending);

        let source = source.to_string();
        let picker = picker.clone();
        let tx = action_tx.clone();

        tokio::task::spawn_blocking(move || {
            let result = render_blocking(source, &picker);
            let entry = match result {
                Ok(protocol) => MermaidEntry::Ready(Box::new(protocol)),
                Err(e) => MermaidEntry::Failed(e),
            };
            let _ = tx.send(crate::action::Action::MermaidReady(id, Box::new(entry)));
        });

        true
    }
}

/// CPU-bound: render mermaid source → SVG → DynamicImage → StatefulProtocol.
fn render_blocking(source: String, picker: &Picker) -> Result<StatefulProtocol, String> {
    let svg = mermaid_rs_renderer::render(&source)
        .map_err(|e| format!("render error: {e}"))?;

    let img = svg_to_image(&svg).map_err(|e| format!("svg rasterize: {e}"))?;
    Ok(picker.new_resize_protocol(img))
}

/// Rasterize an SVG string to a `DynamicImage`.
fn svg_to_image(svg: &str) -> Result<DynamicImage, String> {
    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opts)
        .map_err(|e| format!("usvg parse: {e}"))?;

    let size = tree.size();
    let width = size.width() as u32;
    let height = size.height() as u32;
    if width == 0 || height == 0 {
        return Err("empty SVG dimensions".to_string());
    }

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or("failed to allocate pixmap")?;

    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());

    // tiny_skia's pixmap is RGBA premultiplied; image::RgbaImage is RGBA
    // straight-alpha. Demultiply each pixel.
    let raw = pixmap.take();
    let rgba = demultiply_alpha(raw, width, height)?;
    Ok(DynamicImage::ImageRgba8(rgba))
}

fn demultiply_alpha(data: Vec<u8>, width: u32, height: u32) -> Result<RgbaImage, String> {
    let mut out = Vec::with_capacity(data.len());
    for pixel in data.chunks_exact(4) {
        let (r, g, b, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let factor = 255.0 / a as f32;
            out.push((r as f32 * factor).min(255.0) as u8);
            out.push((g as f32 * factor).min(255.0) as u8);
            out.push((b as f32 * factor).min(255.0) as u8);
            out.push(a);
        }
    }
    RgbaImage::from_raw(width, height, out).ok_or("image buffer size mismatch".to_string())
}

/// Create a [`Picker`] by querying the terminal, or return `None` on failure.
///
/// Returns `None` when inside tmux (detected via the `$TMUX` environment
/// variable) because tmux's multiplexing layer corrupts terminal graphics
/// escape sequences.
pub fn create_picker() -> Option<Picker> {
    if std::env::var("TMUX").is_ok() {
        return None;
    }

    // `from_query_stdio` sends escape sequences to query font-size and graphics
    // protocol support. Fall back to halfblocks if the query fails or the
    // terminal doesn't respond in time.
    match Picker::from_query_stdio() {
        Ok(picker) => Some(picker),
        Err(_) => Some(Picker::halfblocks()),
    }
}

/// The reason graphics are unavailable in a tmux session.
pub const TMUX_DISABLED_REASON: &str = "disable tmux for graphics";
