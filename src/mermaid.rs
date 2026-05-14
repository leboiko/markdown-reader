use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use image::{DynamicImage, RgbaImage};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use resvg::usvg;

use crate::config::{MermaidMode, MermaidTextBackend};
use crate::markdown::MermaidBlockId;

/// Maximum number of concurrent mermaid render tasks.  Each task runs
/// `mermaid_rs_renderer::render()` + resvg rasterization on a blocking
/// thread — both CPU-intensive.  Without a cap, opening a doc with many
/// diagrams after a theme change (which clears the cache) would spawn
/// one thread per diagram and saturate every core.
const MAX_CONCURRENT_RENDERS: u32 = 2;

/// Timeout for a single mermaid render.  `mermaid-rs-renderer` is
/// pre-1.0 and can hang on certain diagram types; without a timeout the
/// blocking thread runs forever at 100% CPU.
const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

/// Global counter of in-flight mermaid render tasks.
static IN_FLIGHT: AtomicU32 = AtomicU32::new(0);

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

/// Hard safety cap for ASCII / Unicode diagram heights. The diagram-text
/// path is deterministic — the rendered height is always the line count of
/// the produced glyphs — so the user-configurable `mermaid_max_height`
/// (which is meaningful for image rasterisation and source-text fallbacks)
/// is bypassed here. This cap exists only to bound the layout against
/// pathological input (e.g. a 100k-line "diagram"), not to constrain
/// normal use.
pub const ASCII_DIAGRAM_HARD_CAP: u32 = 1000;

/// Maximum mermaid block height used in the test suite as an explicit cap.
///
/// Production code uses the user-configurable value from
/// `Config::mermaid_max_height` instead of this constant.
#[allow(dead_code)]
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
    ///
    /// `styled_text_cache` lazily stores the styled `Text<'static>` produced from
    /// the error message so that successive render frames skip the per-line
    /// String/Span allocation.  It is `None` on first access and populated on the
    /// first render call.  The cache is invalidated implicitly when the whole
    /// `MermaidCache` is cleared (theme change or mode switch).
    Failed {
        /// Short error message shown in the footer.
        msg: String,
        /// Lazily-built styled text for the fallback display.
        styled_text_cache: std::cell::RefCell<Option<ratatui::text::Text<'static>>>,
    },
    /// Graphics are disabled (e.g. inside tmux); display the source with a hint.
    ///
    /// `styled_text_cache` has the same semantics as in [`MermaidEntry::Failed`].
    SourceOnly {
        /// Short reason shown in the footer.
        reason: String,
        /// Lazily-built styled text for the fallback display.
        styled_text_cache: std::cell::RefCell<Option<ratatui::text::Text<'static>>>,
    },
    /// Graphics are unavailable but the diagram was successfully rendered
    /// to Unicode box-drawing characters via `figurehead`.  The `String`
    /// contains the ready-to-display ASCII/Unicode art.
    ///
    /// `styled_text_cache` has the same semantics as in [`MermaidEntry::Failed`].
    AsciiDiagram {
        /// The rendered diagram text.
        diagram: String,
        /// Short reason why graphics aren't available (shown in the footer).
        reason: String,
        /// Lazily-built styled text for the fallback display.
        styled_text_cache: std::cell::RefCell<Option<ratatui::text::Text<'static>>>,
    },
}

/// Rendering configuration passed to [`MermaidCache::ensure_queued`].
///
/// Grouping these parameters avoids tripping the `clippy::too_many_arguments`
/// lint while keeping the call site readable.
pub struct MermaidRenderConfig<'a> {
    /// Terminal graphics picker; `None` when graphics are disabled.
    pub picker: Option<&'a ratatui_image::picker::Picker>,
    /// Action channel used to deliver completed render results.
    pub action_tx: &'a tokio::sync::mpsc::UnboundedSender<crate::action::Action>,
    /// Whether the process is running inside a tmux session.
    pub in_tmux: bool,
    /// Background colour used to recolour the rendered SVG.
    pub bg_rgb: (u8, u8, u8),
    /// User-configured rendering mode.
    pub mode: MermaidMode,
    /// User-configured maximum height in display lines.
    pub max_height: u32,
    /// Inner content width of the viewer pane in terminal columns.
    ///
    /// When `Some`, passed to `mermaid_text::render_with_width` so that
    /// text-mode diagrams are compacted to fit within the pane rather than
    /// rendering at their natural (potentially much wider) size.
    /// `None` preserves the previous behaviour (no width budget).
    pub content_width: Option<usize>,
    /// User-configured layered-layout backend for text-mode flowchart and
    /// state diagrams.
    pub text_backend: MermaidTextBackend,
}

/// Per-app cache mapping diagram ids to their render state.
pub struct MermaidCache {
    entries: HashMap<MermaidBlockId, MermaidEntry>,
    /// Heights captured the last time each id had a real entry. Survives
    /// `clear()` so that during the brief window when the cache is being
    /// refreshed (theme change, layout-width change, mode switch), the
    /// `height()` lookup returns the previous height instead of falling back
    /// to `DEFAULT_MERMAID_HEIGHT` and shrinking `total_lines` underneath the
    /// user's cursor.
    last_known_heights: HashMap<MermaidBlockId, u32>,
}

impl MermaidCache {
    /// Create an empty cache with no entries.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_known_heights: HashMap::new(),
        }
    }

    /// Return a shared reference to the entry for `id`, if any.
    pub fn get(&self, id: MermaidBlockId) -> Option<&MermaidEntry> {
        self.entries.get(&id)
    }

    /// Return a mutable reference to the entry for `id`, if any.
    pub fn get_mut(&mut self, id: MermaidBlockId) -> Option<&mut MermaidEntry> {
        self.entries.get_mut(&id)
    }

    /// Insert a new entry, overwriting any existing one.
    pub fn insert(&mut self, id: MermaidBlockId, entry: MermaidEntry) {
        if let Some(h) = entry_height(&entry) {
            self.last_known_heights.insert(id, h);
        }
        self.entries.insert(id, entry);
    }

    /// Return the display-line height for `id` based on its current cache state.
    ///
    /// # Arguments
    ///
    /// * `id`         – diagram identifier.
    /// * `source`     – raw mermaid source (used to measure fallback text height).
    /// * `max_height` – user-configured upper bound (from `Config::mermaid_max_height`).
    ///
    /// # Behaviour
    ///
    /// - `Ready`: the stored `cell_height` derived from the rendered image.
    /// - `Pending`: `MIN_MERMAID_HEIGHT` (small placeholder until rendering finishes).
    /// - `Failed` / `SourceOnly`: source-line count clamped to `[MIN, max_height]`.
    /// - `AsciiDiagram`: diagram-line count clamped to `[MIN, ASCII_DIAGRAM_HARD_CAP]`.
    ///   `mermaid_max_height` is intentionally NOT applied here — text-mode
    ///   diagrams are deterministic content the user explicitly authored;
    ///   truncating them at a small default makes the rest of the diagram
    ///   silently unreachable (the viewer scrolls past it instead of
    ///   exposing more rows). The hard cap is a defensive bound only.
    /// - Not present: `DEFAULT_MERMAID_HEIGHT`.
    pub fn height(&self, id: MermaidBlockId, source: &str, max_height: u32) -> u32 {
        match self.entries.get(&id) {
            None => self
                .last_known_heights
                .get(&id)
                .copied()
                .unwrap_or(DEFAULT_MERMAID_HEIGHT),
            Some(MermaidEntry::Pending) => self
                .last_known_heights
                .get(&id)
                .copied()
                .unwrap_or(MIN_MERMAID_HEIGHT),
            Some(MermaidEntry::Ready { cell_height, .. }) => *cell_height,
            Some(MermaidEntry::Failed { .. } | MermaidEntry::SourceOnly { .. }) => {
                let source_lines = crate::cast::u32_sat(source.lines().count()) + 2;
                source_lines.clamp(MIN_MERMAID_HEIGHT, max_height)
            }
            Some(MermaidEntry::AsciiDiagram { diagram, .. }) => {
                let diagram_lines = crate::cast::u32_sat(diagram.lines().count()) + 2;
                diagram_lines.clamp(MIN_MERMAID_HEIGHT, ASCII_DIAGRAM_HARD_CAP)
            }
        }
    }

    /// Remove all cached entries. `last_known_heights` is preserved so that
    /// in-flight re-renders (theme change, layout-width change) don't shrink
    /// `total_lines` and shift the user's cursor while the new images are
    /// being prepared.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Ensure `id` has an entry. If it already has one, do nothing and return
    /// `false`. If not, create an entry (and possibly spawn a background task),
    /// then return `true` only when a new background image task was spawned.
    ///
    /// # Decision tree
    ///
    /// 1. **`Text` mode** — always use figurehead; never spawn image tasks.
    /// 2. **`has_limited_rendering` types** (e.g. `stateDiagram`) in `Auto` mode —
    ///    try figurehead first; fall back to `SourceOnly` on figurehead error.
    ///    The image pipeline is skipped because mermaid-rs-renderer renders
    ///    these types poorly.
    /// 3. **No graphics** (`picker` is `None`) in `Auto` mode — try figurehead,
    ///    then `SourceOnly`.
    /// 4. **`Image` mode with no graphics** — insert `SourceOnly`; figurehead is
    ///    not tried (the caller explicitly opted out of text fallbacks).
    /// 5. **Graphics available** (`Auto` or `Image` mode) — spawn image render.
    ///
    /// # Arguments
    ///
    /// * `id`     – stable diagram identifier.
    /// * `source` – raw mermaid source text.
    /// * `cfg`    – rendering configuration (mode, picker, max_height, etc.).
    pub fn ensure_queued(
        &mut self,
        id: MermaidBlockId,
        source: &str,
        cfg: &MermaidRenderConfig<'_>,
    ) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }

        // ── Text mode: always figurehead, never spawn image tasks ────────────
        if cfg.mode == MermaidMode::Text {
            let entry = match try_text_render(source, cfg.content_width, cfg.text_backend) {
                Ok(diagram) => MermaidEntry::AsciiDiagram {
                    diagram,
                    reason: "text mode".to_string(),
                    styled_text_cache: std::cell::RefCell::new(None),
                },
                Err(_) => MermaidEntry::SourceOnly {
                    reason: "figurehead render failed, showing source".to_string(),
                    styled_text_cache: std::cell::RefCell::new(None),
                },
            };
            self.entries.insert(id, entry);
            return false;
        }

        // ── Diagram types with limited image-render support ──────────────────
        // In Auto mode we still try figurehead so state diagrams render as
        // Unicode box-drawing art rather than raw source.
        // In Image mode we skip figurehead entirely (caller opted out).
        if has_limited_rendering(source) {
            let entry = if cfg.mode == MermaidMode::Image {
                // Image-only: skip figurehead, show raw source.
                MermaidEntry::SourceOnly {
                    reason: "diagram type not supported by image renderer, showing source"
                        .to_string(),
                    styled_text_cache: std::cell::RefCell::new(None),
                }
            } else {
                // Auto mode: try figurehead first.
                match try_text_render(source, cfg.content_width, cfg.text_backend) {
                    Ok(diagram) => MermaidEntry::AsciiDiagram {
                        diagram,
                        reason: "diagram type uses text-mode rendering".to_string(),
                        styled_text_cache: std::cell::RefCell::new(None),
                    },
                    Err(_) => MermaidEntry::SourceOnly {
                        reason: "diagram type has limited rendering, showing source".to_string(),
                        styled_text_cache: std::cell::RefCell::new(None),
                    },
                }
            };
            self.entries.insert(id, entry);
            return false;
        }

        // ── No graphics available ────────────────────────────────────────────
        let Some(picker) = cfg.picker else {
            let reason = if cfg.in_tmux {
                TMUX_DISABLED_REASON.to_string()
            } else {
                "graphics unavailable".to_string()
            };

            let entry = if cfg.mode == MermaidMode::Image {
                // Image-only mode: don't try figurehead, just show source.
                MermaidEntry::SourceOnly {
                    reason,
                    styled_text_cache: std::cell::RefCell::new(None),
                }
            } else {
                // Auto mode: try text-mode rendering via figurehead before
                // falling back to raw source.  This gives terminals without
                // graphics protocol support a readable Unicode box-drawing diagram.
                match try_text_render(source, cfg.content_width, cfg.text_backend) {
                    Ok(diagram) => MermaidEntry::AsciiDiagram {
                        diagram,
                        reason,
                        styled_text_cache: std::cell::RefCell::new(None),
                    },
                    Err(_) => MermaidEntry::SourceOnly {
                        reason,
                        styled_text_cache: std::cell::RefCell::new(None),
                    },
                }
            };
            self.entries.insert(id, entry);
            return false;
        };

        // ── Graphics available: spawn image render task ──────────────────────
        // Limit concurrent renders to avoid saturating every CPU core when
        // many diagrams are queued (e.g. after a theme change clears the cache).
        if IN_FLIGHT.load(Ordering::Relaxed) >= MAX_CONCURRENT_RENDERS {
            // Don't insert Pending — the block stays un-cached and will be
            // retried on the next draw frame when a slot frees up.
            return false;
        }
        IN_FLIGHT.fetch_add(1, Ordering::Relaxed);

        self.entries.insert(id, MermaidEntry::Pending);

        let source = source.to_string();
        let picker = picker.clone();
        let tx = cfg.action_tx.clone();
        let bg_rgb = cfg.bg_rgb;
        let max_height = cfg.max_height;

        tokio::task::spawn_blocking(move || {
            // Run the actual render in a sub-thread with a timeout so a
            // hung mermaid-rs-renderer doesn't peg the CPU forever.
            let result = render_with_timeout(&source, &picker, bg_rgb, max_height);
            IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
            let entry = match result {
                Ok((protocol, cell_height)) => MermaidEntry::Ready {
                    protocol: Box::new(protocol),
                    cell_height,
                },
                Err(e) => MermaidEntry::Failed {
                    msg: e,
                    styled_text_cache: std::cell::RefCell::new(None),
                },
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

/// Compute the height an entry contributes to `last_known_heights`. Returns
/// `None` for `Pending` (no measured height yet) and for `Failed`/`SourceOnly`
/// (their height depends on the source, computed on demand by `height()`).
fn entry_height(entry: &MermaidEntry) -> Option<u32> {
    match entry {
        MermaidEntry::Ready { cell_height, .. } => Some(*cell_height),
        MermaidEntry::AsciiDiagram { diagram, .. } => {
            let lines = crate::cast::u32_sat(diagram.lines().count()) + 2;
            Some(lines.clamp(MIN_MERMAID_HEIGHT, ASCII_DIAGRAM_HARD_CAP))
        }
        MermaidEntry::Pending | MermaidEntry::Failed { .. } | MermaidEntry::SourceOnly { .. } => {
            None
        }
    }
}

/// Wrapper that runs [`render_blocking`] inside a sub-thread with a
/// [`RENDER_TIMEOUT`] deadline.  If the mermaid parser hangs (known
/// pre-1.0 issue), the sub-thread is detached and the caller gets a
/// clean error instead of a permanently pegged CPU core.
fn render_with_timeout(
    source: &str,
    picker: &Picker,
    bg_rgb: (u8, u8, u8),
    max_height: u32,
) -> Result<(StatefulProtocol, u32), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let source = source.to_string();
    let picker = picker.clone();

    std::thread::spawn(move || {
        let result = render_blocking(&source, &picker, bg_rgb, max_height);
        let _ = tx.send(result);
    });

    rx.recv_timeout(RENDER_TIMEOUT).map_err(|_| {
        format!(
            "mermaid render timed out after {}s — diagram may trigger a parser bug",
            RENDER_TIMEOUT.as_secs()
        )
    })?
}

/// CPU-bound: render mermaid source → SVG → `DynamicImage` → `StatefulProtocol`.
///
/// Returns the protocol and the image's height in terminal cells, clamped to
/// `[MIN_MERMAID_HEIGHT, max_height]`.
///
/// # Arguments
///
/// * `source`     – raw mermaid source text.
/// * `picker`     – terminal graphics picker.
/// * `bg_rgb`     – background colour used to recolour the rendered SVG.
/// * `max_height` – upper bound in display lines (from `Config::mermaid_max_height`).
fn render_blocking(
    source: &str,
    picker: &Picker,
    bg_rgb: (u8, u8, u8),
    max_height: u32,
) -> Result<(StatefulProtocol, u32), String> {
    let svg = mermaid_rs_renderer::render(source).map_err(|e| format!("render error: {e}"))?;

    let img = svg_to_image(&svg, bg_rgb).map_err(|e| format!("svg rasterize: {e}"))?;

    let cell_height = compute_cell_height(&img, picker, max_height);
    Ok((picker.new_resize_protocol(img), cell_height))
}

/// Compute the natural height of `img` in terminal cells using the picker's
/// reported font size. Clamped to `[MIN_MERMAID_HEIGHT, max_height]`.
///
/// # Arguments
///
/// * `img`        – the rasterised diagram image.
/// * `picker`     – terminal graphics picker (provides font cell pixel size).
/// * `max_height` – upper bound in display lines (from `Config::mermaid_max_height`).
fn compute_cell_height(img: &DynamicImage, picker: &Picker, max_height: u32) -> u32 {
    let (_, cell_px_h) = picker.font_size();
    let px_h = img.height();
    if cell_px_h == 0 {
        return DEFAULT_MERMAID_HEIGHT;
    }
    let cells = px_h.div_ceil(u32::from(cell_px_h));
    cells.clamp(MIN_MERMAID_HEIGHT, max_height)
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

    let is_dark = (u16::from(bg_rgb.0) + u16::from(bg_rgb.1) + u16::from(bg_rgb.2)) / 3 < 128;
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
    // SVG dimensions are always non-negative and bounded in practice; `.ceil()` followed
    // by clamping to u32 is intentional — suppress pedantic cast warnings here.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width = (size.width() * SVG_RENDER_SCALE).ceil() as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
    let rgba = demultiply_alpha(&raw, width, height)?;
    Ok(DynamicImage::ImageRgba8(rgba))
}

fn demultiply_alpha(data: &[u8], width: u32, height: u32) -> Result<RgbaImage, String> {
    let mut out = Vec::with_capacity(data.len());
    for pixel in data.chunks_exact(4) {
        let (r, g, b, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            // f32 arithmetic for premultiplied-alpha demultiplication; values are
            // always in [0.0, 255.0] after `.min(255.0)`, so the cast to u8 is safe.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                let factor = 255.0 / f32::from(a);
                out.push((f32::from(r) * factor).min(255.0) as u8);
                out.push((f32::from(g) * factor).min(255.0) as u8);
                out.push((f32::from(b) * factor).min(255.0) as u8);
            }
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

/// Map a [`MermaidTextBackend`] config value to its [`mermaid_text`] equivalent.
///
/// `Auto` is resolved against the source: when the source contains a
/// `subgraph` block with an inner `direction` override (the shape that
/// Sugiyama is documented to render less compactly than Native — see
/// `RenderOptions::backend` in `mermaid-text`), `Auto` resolves to
/// `Native`.  Every other shape resolves to `Sugiyama`.
///
/// The choice is conservative on purpose: Sugiyama is the library's default
/// and handles almost every diagram cleanly, so `Auto` only deviates from it
/// when there is a documented reason to.
fn to_layout_backend(
    backend: MermaidTextBackend,
    source: &str,
) -> mermaid_text::layout::LayoutBackend {
    match backend {
        MermaidTextBackend::Sugiyama => mermaid_text::layout::LayoutBackend::Sugiyama,
        MermaidTextBackend::Native => mermaid_text::layout::LayoutBackend::Native,
        MermaidTextBackend::Auto => {
            if auto_prefers_native(source) {
                mermaid_text::layout::LayoutBackend::Native
            } else {
                mermaid_text::layout::LayoutBackend::Sugiyama
            }
        }
    }
}

/// True when the source contains a `subgraph` block that has its own
/// `direction` line — the shape where Sugiyama is less compact than Native.
///
/// Detection is purely lexical: we scan the lines, increment a depth counter
/// on each `subgraph` opener, decrement on `end`, and look for `direction`
/// anywhere inside a non-zero-depth region.  This avoids parsing the source
/// twice (mermaid-text would otherwise parse it again immediately after) and
/// is robust to indentation, comments, and the various keyword forms the
/// flowchart parser accepts.
fn auto_prefers_native(source: &str) -> bool {
    let mut depth: usize = 0;
    for line in source.lines() {
        let trimmed = line.trim_start();
        // `subgraph FOO`, `subgraph "Foo Bar"`, or a bare `subgraph`. Match
        // the keyword with a trailing boundary so identifiers that happen to
        // start with `subgraph` aren't caught.
        if let Some(rest) = trimmed.strip_prefix("subgraph")
            && (rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            depth += 1;
            continue;
        }
        if depth == 0 {
            continue;
        }
        // `end` (alone, or followed by whitespace / comment) closes the
        // innermost subgraph.  Edge labels like `endorse` must not match.
        if trimmed == "end"
            || trimmed.starts_with("end ")
            || trimmed.starts_with("end\t")
            || trimmed.starts_with("end%")
        {
            depth -= 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("direction")
            && rest.starts_with(char::is_whitespace)
        {
            return true;
        }
    }
    false
}

/// Render mermaid source to Unicode box-drawing text via `mermaid-text`.
///
/// `mermaid-text` is our own library crate (MIT, zero unsafe, no stdout
/// writes, no panics on valid input).  Currently supports flowcharts
/// (`graph`/`flowchart` with LR/TD/RL/BT).  Unsupported diagram types
/// return `Err` and fall back to showing raw source.
///
/// # Arguments
///
/// * `source` – raw mermaid source text.
/// * `max_width` – optional column budget; when `Some(w)` the renderer
///   progressively compacts gap sizes until the output fits within `w`
///   columns.  `None` renders at natural size (previous behaviour).
/// * `backend` – the layered-layout backend to use for flowchart and state
///   diagrams. Other diagram types ignore this.
fn try_text_render(
    source: &str,
    max_width: Option<usize>,
    backend: MermaidTextBackend,
) -> Result<String, String> {
    let opts = mermaid_text::RenderOptions {
        max_width,
        backend: to_layout_backend(backend, source),
        ..Default::default()
    };
    mermaid_text::render_with_options(source, &opts).map_err(|e| format!("{e}"))
}

/// Public wrapper around [`try_text_render`] for use from the
/// `MermaidReady` action handler when an image render fails.
///
/// # Arguments
///
/// * `source` – raw mermaid source text.
/// * `max_width` – optional column budget passed to `mermaid_text`.
/// * `backend` – the layered-layout backend to use; see [`try_text_render`].
pub fn try_text_render_public(
    source: &str,
    max_width: Option<usize>,
    backend: MermaidTextBackend,
) -> Result<String, String> {
    try_text_render(source, max_width, backend)
}

/// Render mermaid source with an explicit `(layer_gap, node_gap)` override,
/// bypassing the `max_width` compaction pipeline. Used by the full-screen
/// mermaid modal's `+`/`-` zoom keys so each press maps to a deterministic
/// layout step (no "stuck on a discrete compaction level" surprises).
///
/// Sequence, pie, and erDiagram diagrams ignore the gap override —
/// they have their own layout pipelines. For those types the result is the
/// same as [`try_text_render_public`] with `max_width = None`.
pub fn try_text_render_with_gaps(
    source: &str,
    layer_gap: usize,
    node_gap: usize,
    backend: MermaidTextBackend,
) -> Result<String, String> {
    let opts = mermaid_text::RenderOptions {
        gaps_override: Some((layer_gap, node_gap)),
        backend: to_layout_backend(backend, source),
        ..Default::default()
    };
    mermaid_text::render_with_options(source, &opts).map_err(|e| format!("{e}"))
}

fn has_limited_rendering(source: &str) -> bool {
    let t = source.trim_start();
    t.starts_with("stateDiagram")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEQUENCE_DIAGRAM: &str = r"sequenceDiagram
    participant W as Worker
    participant CP as CheckpointStore
    participant ES as EventReader
    W->>CP: Read checkpoint (last sequence)
    CP-->>W: sequence_number
    W->>ES: Poll events (after sequence, limit 500)
    ES-->>W: batch of StoredEvents";

    const GRAPH_LR_1: &str = r"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat every cycle| HB[Heartbeat]
    HB -->|checked every 10s| WD[Watchdog]
    WD -->|stall > 120s| CT[Cancel Token]
    CT -->|stops| W
    style WD fill:#c82,stroke:#fff,color:#fff";

    const STATE_DIAGRAM: &str = r"stateDiagram-v2
    [*] --> CLOSED
    CLOSED --> OPEN : 5 consecutive failures
    OPEN --> HALF_OPEN : probe interval elapsed
    HALF_OPEN --> CLOSED : probe succeeds
    HALF_OPEN --> OPEN : probe fails (increased backoff)";

    const GRAPH_LR_2: &str = r"graph LR
    subgraph projections-pg [projections-pg :9092]
        PG_W[event_log, account_registry]
    end
    PG_W --> PG[(PostgreSQL)]
    style PG fill:#336,stroke:#fff,color:#fff";

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

    /// A helper used by tests that need to call `ensure_queued` without a real
    /// tokio runtime. The channel is created but never polled; we only care about
    /// the resulting cache entry, not whether actions are delivered.
    fn make_tx() -> tokio::sync::mpsc::UnboundedSender<crate::action::Action> {
        tokio::sync::mpsc::unbounded_channel().0
    }

    #[test]
    fn cache_height_no_entry_returns_default() {
        let cache = MermaidCache::new();
        let id = MermaidBlockId(1);
        assert_eq!(
            cache.height(id, "graph LR\n    A --> B", MAX_MERMAID_HEIGHT),
            DEFAULT_MERMAID_HEIGHT
        );
    }

    #[test]
    fn cache_height_pending_returns_min() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(2);
        cache.insert(id, MermaidEntry::Pending);
        assert_eq!(cache.height(id, "", MAX_MERMAID_HEIGHT), MIN_MERMAID_HEIGHT);
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
        assert_eq!(cache.height(id, "", MAX_MERMAID_HEIGHT), 15);
    }

    #[test]
    fn cache_height_failed_clamps_to_range() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(4);
        cache.insert(
            id,
            MermaidEntry::Failed {
                msg: "err".to_string(),
                styled_text_cache: std::cell::RefCell::new(None),
            },
        );
        let h = cache.height(id, "line1\nline2\nline3", MAX_MERMAID_HEIGHT);
        assert!((MIN_MERMAID_HEIGHT..=MAX_MERMAID_HEIGHT).contains(&h));
    }

    #[test]
    fn cache_height_source_only_clamps_to_range() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(5);
        cache.insert(
            id,
            MermaidEntry::SourceOnly {
                reason: "tmux".to_string(),
                styled_text_cache: std::cell::RefCell::new(None),
            },
        );
        let mut source = String::new();
        for i in 0..100usize {
            source.push_str("line");
            source.push_str(&i.to_string());
            source.push('\n');
        }
        let h = cache.height(id, &source, MAX_MERMAID_HEIGHT);
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

        assert!(cache.get(id1).is_some());
        assert!(cache.get(id2).is_none());
        assert!(cache.get(id3).is_some());
    }

    /// In `Auto` mode, a `stateDiagram-v2` source must produce an
    /// `AsciiDiagram` entry — figurehead handles state diagrams and the
    /// image pipeline is skipped for them.
    #[test]
    fn limited_rendering_uses_figurehead() {
        // `stateDiagram` is treated as a "limited rendering" type — the
        // image pipeline (mermaid-rs-renderer SVG → raster) handles it
        // poorly, so the cache always routes through figurehead
        // (mermaid-text). With mermaid-text 0.6.0+ this now succeeds and
        // we get an AsciiDiagram. (Pre-0.6.0 it failed and fell back to
        // SourceOnly — the test originally pinned that fallback.)
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(100);
        let src = "stateDiagram-v2\n[*] --> A\nA --> B";
        let tx = make_tx();

        let cfg = MermaidRenderConfig {
            picker: None,
            action_tx: &tx,
            in_tmux: false,
            bg_rgb: (0, 0, 0),
            mode: MermaidMode::Auto,
            max_height: 30,
            content_width: None,
            text_backend: MermaidTextBackend::default(),
        };
        cache.ensure_queued(id, src, &cfg);

        let entry = cache.get(id).expect("entry must be present");
        assert!(
            matches!(entry, MermaidEntry::AsciiDiagram { .. }),
            "expected AsciiDiagram from figurehead (mermaid-text 0.6.0+ supports stateDiagram)"
        );
    }

    /// In `Text` mode, a flowchart must not spawn an image task and
    /// must produce an `AsciiDiagram` via figurehead.
    #[test]
    fn text_mode_never_spawns_image_task() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(101);
        let src = "graph LR\n    A --> B";
        let tx = make_tx();

        let picker = ratatui_image::picker::Picker::halfblocks();
        let cfg = MermaidRenderConfig {
            picker: Some(&picker),
            action_tx: &tx,
            in_tmux: false,
            bg_rgb: (0, 0, 0),
            mode: MermaidMode::Text,
            max_height: 30,
            content_width: None,
            text_backend: MermaidTextBackend::default(),
        };
        let spawned = cache.ensure_queued(id, src, &cfg);

        assert!(!spawned, "Text mode must never spawn an image task");
        let entry = cache.get(id).expect("entry must be present");
        // mermaid-text handles flowcharts → AsciiDiagram.
        assert!(
            matches!(entry, MermaidEntry::AsciiDiagram { .. }),
            "expected AsciiDiagram from mermaid-text in Text mode"
        );
    }

    /// In `Image` mode with no picker, the entry must be `SourceOnly` — figurehead
    /// is not tried because the caller opted out of text fallbacks.
    #[test]
    fn image_mode_skips_figurehead() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(102);
        let src = "graph LR\n    A --> B";
        let tx = make_tx();

        let cfg = MermaidRenderConfig {
            picker: None,
            action_tx: &tx,
            in_tmux: false,
            bg_rgb: (0, 0, 0),
            mode: MermaidMode::Image,
            max_height: 30,
            content_width: None,
            text_backend: MermaidTextBackend::default(),
        };
        cache.ensure_queued(id, src, &cfg);

        let entry = cache.get(id).expect("entry must be present");
        assert!(
            matches!(entry, MermaidEntry::SourceOnly { .. }),
            "expected SourceOnly in Image mode with no graphics, got a different variant"
        );
    }

    #[test]
    fn ascii_diagram_height_ignores_max_height() {
        // A 60-line text-mode diagram (e.g. a state diagram with composite
        // states) must reserve all 60+ lines in the document layout, even
        // when `mermaid_max_height` is the default 30. Truncating here
        // makes the bottom of the diagram silently unreachable: the
        // viewer scrolls past the reserved region into the next block.
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(300);
        let diagram: String = (0..60).map(|i| format!("line {i}\n")).collect();
        cache.insert(
            id,
            MermaidEntry::AsciiDiagram {
                diagram,
                reason: "test".to_string(),
                styled_text_cache: std::cell::RefCell::new(None),
            },
        );
        let h = cache.height(id, "irrelevant", 30);
        assert_eq!(
            h, 62,
            "AsciiDiagram must reserve the full diagram height (60 + 2 padding), \
             not the user's mermaid_max_height clamp"
        );
    }

    #[test]
    fn ascii_diagram_height_caps_at_safety_bound() {
        // Defensive cap so a pathological 100k-line "diagram" can't blow up
        // the document layout.
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(301);
        let diagram: String = (0..(ASCII_DIAGRAM_HARD_CAP as usize + 50))
            .map(|_| "x\n")
            .collect();
        cache.insert(
            id,
            MermaidEntry::AsciiDiagram {
                diagram,
                reason: "test".to_string(),
                styled_text_cache: std::cell::RefCell::new(None),
            },
        );
        let h = cache.height(id, "", 30);
        assert_eq!(h, ASCII_DIAGRAM_HARD_CAP);
    }

    /// Pinning that the `text_backend` argument actually reaches the
    /// underlying `mermaid_text::RenderOptions`.
    ///
    /// `Native` and `Sugiyama` produce visibly different text-mode layouts
    /// for the resilience-graph fixture (`GRAPH_LR_1` has a subgraph with
    /// nested-direction overrides — a shape Sugiyama renders less compactly
    /// than the in-house Native pipeline). If this test ever passes with
    /// equal outputs it means the `text_backend` parameter is being threaded
    /// through the function signature but discarded before reaching
    /// `RenderOptions::backend` — i.e. the entire feature is silently
    /// no-op'd. None of the surrounding plumbing tests catch that.
    #[test]
    fn backend_threads_through_render_with_options() {
        let native = try_text_render_public(GRAPH_LR_1, None, MermaidTextBackend::Native)
            .expect("Native render must succeed");
        let sugiyama = try_text_render_public(GRAPH_LR_1, None, MermaidTextBackend::Sugiyama)
            .expect("Sugiyama render must succeed");
        assert_ne!(
            native, sugiyama,
            "Native and Sugiyama outputs must differ — if equal, the \
             text_backend argument is being discarded before reaching \
             RenderOptions::backend (the whole feature is a no-op)"
        );
    }

    /// Same load-bearing check for the modal `+`/`-` zoom path. The modal
    /// builds its own `RenderOptions` with `gaps_override` set, so it could
    /// silently fall back to the default backend if the parameter is dropped
    /// at the wrapper boundary. Pin the difference here too so a regression
    /// in either wrapper is caught immediately.
    #[test]
    fn modal_gap_render_uses_chosen_backend() {
        let native = try_text_render_with_gaps(GRAPH_LR_1, 6, 2, MermaidTextBackend::Native)
            .expect("Native gap render must succeed");
        let sugiyama = try_text_render_with_gaps(GRAPH_LR_1, 6, 2, MermaidTextBackend::Sugiyama)
            .expect("Sugiyama gap render must succeed");
        assert_ne!(
            native, sugiyama,
            "modal `+`/`-` zoom path must honour the user's text_backend \
             choice — if equal, the argument is being dropped at the \
             try_text_render_with_gaps wrapper boundary"
        );
    }

    // ── Auto backend resolution ──────────────────────────────────────────────

    /// `auto_prefers_native` must detect the documented Sugiyama coverage
    /// gap: a subgraph block that has an inner `direction` line.
    #[test]
    fn auto_prefers_native_when_subgraph_has_inner_direction() {
        // GRAPH_LR_1 is the resilience fixture used by
        // `backend_threads_through_render_with_options`. Its `Supervisor`
        // subgraph carries `direction TB` inside an outer `graph LR` — the
        // exact shape that the test docstring above (and the scope doc at
        // docs/scope-mermaid-backend-selection.md) call out as Sugiyama's
        // weak spot.
        assert!(
            auto_prefers_native(GRAPH_LR_1),
            "GRAPH_LR_1 has a subgraph with `direction TB` inside `graph LR` \
             — the load-bearing fixture for this heuristic. If this fails, \
             the detection regressed or the fixture was edited."
        );
    }

    /// A flat dependency graph (no subgraphs) must NOT trigger the Native
    /// override — Auto should resolve to Sugiyama for the common case.
    #[test]
    fn auto_does_not_prefer_native_for_flat_dag() {
        let src = "graph LR\n    A --> B\n    B --> C\n    C --> D\n";
        assert!(
            !auto_prefers_native(src),
            "flat LR chains must take the Sugiyama path under Auto"
        );
    }

    /// A subgraph WITHOUT an inner `direction` line must NOT trigger Native
    /// — the heuristic is intentionally narrow.  Pinning this prevents the
    /// detection from drifting into "any subgraph triggers Native" (which
    /// would flip far too many gallery snapshots).
    #[test]
    fn auto_does_not_prefer_native_for_plain_subgraph() {
        let src = "graph LR\n\
                   subgraph cluster\n\
                       A --> B\n\
                   end\n\
                   B --> C\n";
        assert!(
            !auto_prefers_native(src),
            "subgraph without inner `direction` must NOT trigger Native — \
             only the documented Sugiyama coverage gap (subgraph + inner \
             direction) should opt in"
        );
    }

    /// A nested `direction` inside a `subgraph` block (regardless of which
    /// flow direction) must trigger Native. This pins the keyword
    /// detection logic, not the specific direction value.
    #[test]
    fn auto_prefers_native_for_any_nested_direction_value() {
        for dir in ["TB", "BT", "LR", "RL"] {
            let src = format!(
                "graph LR\n\
                 subgraph cluster\n\
                     direction {dir}\n\
                     A --> B\n\
                 end\n"
            );
            assert!(
                auto_prefers_native(&src),
                "inner `direction {dir}` must trigger Native"
            );
        }
    }

    /// An edge label or node id that starts with the substring "direction"
    /// (e.g. an unfortunate variable name) must NOT trigger Native — the
    /// detection requires the keyword to be followed by whitespace.
    #[test]
    fn auto_detection_requires_keyword_boundary() {
        // `directional --> A` would be a regression if we matched substrings;
        // pin that the detection is keyword-bounded.
        let src = "graph LR\n\
                   subgraph cluster\n\
                       directional --> A\n\
                   end\n";
        assert!(
            !auto_prefers_native(src),
            "an identifier starting with `direction` must NOT be treated \
             as the `direction` keyword"
        );
    }

    /// `Auto` must produce the same bytes as `Sugiyama` for the flat-DAG
    /// case AND the same bytes as `Native` for the subgraph-with-inner-
    /// direction case.  Strongest assertion possible: it pins that Auto
    /// is genuinely routing, not just defaulting either way.
    #[test]
    fn auto_routes_to_expected_backend() {
        let flat = "graph LR\n    A --> B --> C\n";
        let auto_flat = try_text_render_public(flat, None, MermaidTextBackend::Auto)
            .expect("Auto flat render must succeed");
        let sugiyama_flat = try_text_render_public(flat, None, MermaidTextBackend::Sugiyama)
            .expect("Sugiyama flat render must succeed");
        assert_eq!(
            auto_flat, sugiyama_flat,
            "Auto must produce Sugiyama output for a flat LR chain"
        );

        let auto_subgraph = try_text_render_public(GRAPH_LR_1, None, MermaidTextBackend::Auto)
            .expect("Auto subgraph render must succeed");
        let native_subgraph = try_text_render_public(GRAPH_LR_1, None, MermaidTextBackend::Native)
            .expect("Native subgraph render must succeed");
        assert_eq!(
            auto_subgraph, native_subgraph,
            "Auto must produce Native output for a subgraph with inner \
             `direction TB` — the documented Sugiyama coverage gap"
        );
    }

    #[test]
    fn height_respects_custom_max_height() {
        let mut cache = MermaidCache::new();
        let id = MermaidBlockId(200);
        // 50 source lines + 2 = 52; should be clamped to 25.
        let source: String = (0..50).map(|i| format!("line{i}\n")).collect();
        cache.insert(
            id,
            MermaidEntry::SourceOnly {
                reason: "x".to_string(),
                styled_text_cache: std::cell::RefCell::new(None),
            },
        );
        let h = cache.height(id, &source, 25);
        assert_eq!(h, 25, "height must be clamped to the supplied max_height");
    }
}
