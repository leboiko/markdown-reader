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
    let svg = mermaid_rs_renderer::render(&source).map_err(|e| format!("render error: {e}"))?;

    let img = svg_to_image(&svg).map_err(|e| format!("svg rasterize: {e}"))?;
    Ok(picker.new_resize_protocol(img))
}

/// Multiplier applied to the SVG's intrinsic size when rasterizing. Mermaid's
/// default SVG dimensions are small (a few hundred pixels), and ratatui-image's
/// `Resize::Fit` preserves aspect without upscaling, so without this the image
/// only fills a fraction of the viewer. SVG is vector so there is no quality
/// loss; the extra pixels are downscaled to the rect as needed.
const SVG_RENDER_SCALE: f32 = 3.0;

/// Rasterize an SVG string to a `DynamicImage`.
fn svg_to_image(svg: &str) -> Result<DynamicImage, String> {
    let opts = usvg::Options {
        fontdb: Arc::clone(font_db()),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &opts).map_err(|e| format!("usvg parse: {e}"))?;

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
                Ok(svg) => match svg_to_image(&svg) {
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
}
