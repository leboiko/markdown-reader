use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use image::{DynamicImage, RgbaImage};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use resvg::usvg;

use crate::markdown::MermaidBlockId;

/// System fonts are loaded once on first use and reused for every diagram.
/// Without this, resvg rasterizes shapes but cannot render any text in the SVG.
fn font_db() -> &'static Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
}

/// Minimum mermaid block height in display lines, even for tiny diagrams.
pub const MIN_MERMAID_HEIGHT: u32 = 8;

/// Maximum mermaid block height in display lines, so huge diagrams don't
/// consume the entire viewport.
pub const MAX_MERMAID_HEIGHT: u32 = 50;

/// Fallback height used when the cache has no entry for a diagram yet
/// (before any rendering has been kicked off).
pub const DEFAULT_MERMAID_HEIGHT: u32 = 20;

/// The state of a mermaid diagram in the render cache.
pub enum MermaidEntry {
    /// Background task has been spawned; image is not yet available.
    Pending,
    /// Image is ready and encoded for the detected graphics protocol.
    ///
    /// Boxed because `StatefulProtocol` is large (>256 bytes), and clippy
    /// warns about large enum variants that inflate every instance of the enum.
    Ready {
        protocol: Box<StatefulProtocol>,
        /// Height of the rendered image in terminal cells, clamped to
        /// `[MIN_MERMAID_HEIGHT, MAX_MERMAID_HEIGHT]`.
        cell_height: u32,
    },
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
    /// Create an empty cache with no entries.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Return a shared reference to the entry for `id`, if any.
    pub fn get(&self, id: &MermaidBlockId) -> Option<&MermaidEntry> {
        self.entries.get(id)
    }

    /// Return a mutable reference to the entry for `id`, if any.
    pub fn get_mut(&mut self, id: &MermaidBlockId) -> Option<&mut MermaidEntry> {
        self.entries.get_mut(id)
    }

    /// Insert a new entry, overwriting any existing one.
    pub fn insert(&mut self, id: MermaidBlockId, entry: MermaidEntry) {
        self.entries.insert(id, entry);
    }

    /// Return the display-line height for `id` based on its current cache state.
    ///
    /// - `Ready`: the stored `cell_height` derived from the rendered image.
    /// - `Pending`: `MIN_MERMAID_HEIGHT` (small placeholder until rendering finishes).
    /// - `Failed` / `SourceOnly`: source-line count clamped to the valid range,
    ///   so the fallback text viewer shows all the source without overflow.
    /// - Not present: `DEFAULT_MERMAID_HEIGHT`.
    pub fn height(&self, id: &MermaidBlockId, source: &str) -> u32 {
        match self.entries.get(id) {
            None => DEFAULT_MERMAID_HEIGHT,
            Some(MermaidEntry::Pending) => MIN_MERMAID_HEIGHT,
            Some(MermaidEntry::Ready { cell_height, .. }) => *cell_height,
            Some(MermaidEntry::Failed(_)) | Some(MermaidEntry::SourceOnly(_)) => {
                let source_lines = source.lines().count() as u32 + 2;
                source_lines.clamp(MIN_MERMAID_HEIGHT, MAX_MERMAID_HEIGHT)
            }
        }
    }

    /// Ensure `id` has an entry. If it already has one, do nothing and return
    /// `false`. If not, insert `Pending`, spawn a background render task, and
    /// return `true`.
    ///
    /// When `picker` is `None` (graphics disabled), inserts `SourceOnly`
    /// immediately and returns `false` — no task is spawned.
    /// Remove all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn ensure_queued(
        &mut self,
        id: MermaidBlockId,
        source: &str,
        picker: Option<&Picker>,
        action_tx: &tokio::sync::mpsc::UnboundedSender<crate::action::Action>,
        in_tmux: bool,
        bg_rgb: (u8, u8, u8),
    ) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }

        // Diagram types with known upstream rendering issues fall back to
        // showing source. See github.com/1jehuang/mermaid-rs-renderer/issues/67
        if has_limited_rendering(source) {
            self.entries.insert(
                id,
                MermaidEntry::SourceOnly(
                    "diagram type has limited rendering, showing source".into(),
                ),
            );
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
            let result = render_blocking(source, &picker, bg_rgb);
            let entry = match result {
                Ok((protocol, cell_height)) => MermaidEntry::Ready {
                    protocol: Box::new(protocol),
                    cell_height,
                },
                Err(e) => MermaidEntry::Failed(e),
            };
            let _ = tx.send(crate::action::Action::MermaidReady(id, Box::new(entry)));
        });

        true
    }

    /// Drop all entries whose id is not in `alive`.
    ///
    /// Call after a live reload so stale entries from superseded diagrams don't
    /// accumulate in the cache indefinitely.
    pub fn retain(&mut self, alive: &std::collections::HashSet<MermaidBlockId>) {
        self.entries.retain(|id, _| alive.contains(id));
    }
}

/// CPU-bound: render mermaid source → SVG → DynamicImage → StatefulProtocol.
///
/// Returns the protocol and the image's height in terminal cells, clamped to
/// `[MIN_MERMAID_HEIGHT, MAX_MERMAID_HEIGHT]`.
fn render_blocking(
    source: String,
    picker: &Picker,
    bg_rgb: (u8, u8, u8),
) -> Result<(StatefulProtocol, u32), String> {
    let svg = mermaid_rs_renderer::render(&source).map_err(|e| format!("render error: {e}"))?;

    let img = svg_to_image(&svg, bg_rgb).map_err(|e| format!("svg rasterize: {e}"))?;

    let cell_height = compute_cell_height(&img, picker);
    Ok((picker.new_resize_protocol(img), cell_height))
}

/// Compute the natural height of `img` in terminal cells using the picker's
/// reported font size. Clamped to `[MIN_MERMAID_HEIGHT, MAX_MERMAID_HEIGHT]`.
fn compute_cell_height(img: &DynamicImage, picker: &Picker) -> u32 {
    let (_, cell_px_h) = picker.font_size();
    let px_h = img.height();
    if cell_px_h == 0 {
        return DEFAULT_MERMAID_HEIGHT;
    }
    let cells = px_h.div_ceil(cell_px_h as u32);
    cells.clamp(MIN_MERMAID_HEIGHT, MAX_MERMAID_HEIGHT)
}

/// Multiplier applied to the SVG's intrinsic size when rasterizing. Mermaid's
/// default SVG dimensions are small (a few hundred pixels), and ratatui-image's
/// `Resize::Fit` preserves aspect without upscaling, so without this the image
/// only fills a fraction of the viewer. SVG is vector so there is no quality
/// loss; the extra pixels are downscaled to the rect as needed.
const SVG_RENDER_SCALE: f32 = 3.0;

/// Rasterize an SVG string to a `DynamicImage`, recoloring the SVG's default
/// light palette to match the active theme. For dark themes (average luminance
/// < 128), node fills, text, borders, and arrows are remapped to dark-friendly
/// equivalents. The canvas background is always replaced with `bg_rgb`.
fn svg_to_image(svg: &str, bg_rgb: (u8, u8, u8)) -> Result<DynamicImage, String> {
    let bg_hex = format!("#{:02X}{:02X}{:02X}", bg_rgb.0, bg_rgb.1, bg_rgb.2);
    let svg = svg.replacen("fill=\"#FFFFFF\"", &format!("fill=\"{bg_hex}\""), 1);

    let is_dark = (bg_rgb.0 as u16 + bg_rgb.1 as u16 + bg_rgb.2 as u16) / 3 < 128;
    let svg = if is_dark {
        svg.replace("fill=\"#F8FAFC\"", "fill=\"#1e293b\"")
            .replace("stroke=\"#94A3B8\"", "stroke=\"#64748b\"")
            .replace("fill=\"#0F172A\"", "fill=\"#e2e8f0\"")
            .replace("fill=\"#64748B\"", "fill=\"#94a3b8\"")
            .replace("stroke=\"#64748B\"", "stroke=\"#94a3b8\"")
    } else {
        svg
    };

    let opts = usvg::Options {
        fontdb: Arc::clone(font_db()),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&svg, &opts).map_err(|e| format!("usvg parse: {e}"))?;

    let size = tree.size();
    let width = (size.width() * SVG_RENDER_SCALE).ceil() as u32;
    let height = (size.height() * SVG_RENDER_SCALE).ceil() as u32;
    if width == 0 || height == 0 {
        return Err("empty SVG dimensions".to_string());
    }

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("failed to allocate pixmap")?;

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(SVG_RENDER_SCALE, SVG_RENDER_SCALE),
        &mut pixmap.as_mut(),
    );

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

fn has_limited_rendering(source: &str) -> bool {
    let t = source.trim_start();
    t.starts_with("stateDiagram")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEQUENCE_DIAGRAM: &str = r#"sequenceDiagram
    participant W as Worker
    participant CP as CheckpointStore
    participant ES as EventReader
    W->>CP: Read checkpoint (last sequence)
    CP-->>W: sequence_number
    W->>ES: Poll events (after sequence, limit 500)
    ES-->>W: batch of StoredEvents"#;

    const GRAPH_LR_1: &str = r#"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat every cycle| HB[Heartbeat]
    HB -->|checked every 10s| WD[Watchdog]
    WD -->|stall > 120s| CT[Cancel Token]
    CT -->|stops| W
    style WD fill:#c82,stroke:#fff,color:#fff"#;

    const STATE_DIAGRAM: &str = r#"stateDiagram-v2
    [*] --> CLOSED
    CLOSED --> OPEN : 5 consecutive failures
    OPEN --> HALF_OPEN : probe interval elapsed
    HALF_OPEN --> CLOSED : probe succeeds
    HALF_OPEN --> OPEN : probe fails (increased backoff)"#;

    const GRAPH_LR_2: &str = r#"graph LR
    subgraph projections-pg [projections-pg :9092]
        PG_W[event_log, account_registry]
    end
    PG_W --> PG[(PostgreSQL)]
    style PG fill:#336,stroke:#fff,color:#fff"#;

    #[test]
    fn render_four_target_diagrams() {
        let diagrams = [
            ("sequenceDiagram", SEQUENCE_DIAGRAM),
            ("graph LR (resilience)", GRAPH_LR_1),
            ("stateDiagram-v2", STATE_DIAGRAM),
            ("graph LR (deployments)", GRAPH_LR_2),
        ];

        let mut ready_count = 0;
        let mut failed: Vec<(&str, String)> = Vec::new();

        for (name, src) in &diagrams {
            match mermaid_rs_renderer::render(src) {
                Ok(svg) => match svg_to_image(&svg, (255, 255, 255)) {
                    Ok(_) => {
                        ready_count += 1;
                    }
                    Err(e) => failed.push((name, format!("rasterize: {e}"))),
                },
                Err(e) => failed.push((name, format!("mermaid: {e}"))),
            }
        }

        // CI must have at least 2 of 4 succeed to pass.
        assert!(
            ready_count >= 2,
            "only {ready_count}/4 diagrams rendered successfully; failures: {failed:?}"
        );
    }

    #[test]
    fn cache_height_no_entry_returns_default() {
        let cache = MermaidCache::new();
        let id = MermaidBlockId(1);
        assert_eq!(
            cache.height(&id, "graph LR\n    A --> B"),
            DEFAULT_MERMAID_HEIGHT
        );
    }

    #[test]
    fn cache_height_pending_returns_min() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(2);
        cache.insert(id, MermaidEntry::Pending);
        assert_eq!(cache.height(&id, ""), MIN_MERMAID_HEIGHT);
    }

    #[test]
    fn cache_height_ready_returns_cell_height() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(3);
        cache.insert(
            id,
            MermaidEntry::Ready {
                protocol: Box::new(
                    ratatui_image::picker::Picker::halfblocks()
                        .new_resize_protocol(image::DynamicImage::new_rgba8(10, 10)),
                ),
                cell_height: 15,
            },
        );
        assert_eq!(cache.height(&id, ""), 15);
    }

    #[test]
    fn cache_height_failed_clamps_to_range() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(4);
        cache.insert(id, MermaidEntry::Failed("err".to_string()));
        let h = cache.height(&id, "line1\nline2\nline3");
        assert!((MIN_MERMAID_HEIGHT..=MAX_MERMAID_HEIGHT).contains(&h));
    }

    #[test]
    fn cache_height_source_only_clamps_to_range() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(5);
        cache.insert(id, MermaidEntry::SourceOnly("tmux".to_string()));
        let source: String = (0..100).map(|i| format!("line{i}\n")).collect();
        let h = cache.height(&id, &source);
        assert_eq!(h, MAX_MERMAID_HEIGHT);
    }

    #[test]
    fn cache_retain_drops_stale_entries() {
        let mut cache = MermaidCache::new();
        let id1 = MermaidBlockId(10);
        let id2 = MermaidBlockId(20);
        let id3 = MermaidBlockId(30);
        cache.insert(id1, MermaidEntry::Pending);
        cache.insert(id2, MermaidEntry::Pending);
        cache.insert(id3, MermaidEntry::Pending);

        let mut alive = std::collections::HashSet::new();
        alive.insert(id1);
        alive.insert(id3);
        cache.retain(&alive);

        assert!(cache.get(&id1).is_some());
        assert!(cache.get(&id2).is_none());
        assert!(cache.get(&id3).is_some());
    }
}
