//! High-fidelity LaTeX rendering for block math (`$$…$$`).
//!
//! [`crate::markdown::math::latex_to_unicode`] flattens a formula onto a single
//! text baseline, which is all a terminal can do without graphics. That loses
//! the two-dimensional structure that carries the meaning: a nested fraction
//! becomes `((a)/(b))/(c)`, a matrix loses its rows, and limits stop sitting
//! under their `∑`.
//!
//! This module typesets the formula properly with [RaTeX] — a pure-Rust,
//! KaTeX-compatible engine — rasterises it to a PNG, and hands it to
//! `ratatui-image` to draw with whatever graphics protocol the terminal
//! supports. Fonts are compiled in (`embed-fonts`), so there is nothing to
//! install and no runtime font lookup.
//!
//! [RaTeX]: https://github.com/erweixin/RaTeX
//!
//! # Relationship to the mermaid pipeline
//!
//! The shape deliberately mirrors [`crate::mermaid`]: a cache keyed by a
//! content hash, entries that are `Pending` until a background task delivers a
//! `Ready` protocol, a `last_known_heights` map so the layout does not jump
//! while a re-render is in flight, and a text fallback for every failure path.
//! The differences are that RaTeX is fast and total (a formula either parses or
//! returns an error in about a millisecond), so there is no render timeout, and
//! that the fallback is the Unicode box this crate already drew — meaning a
//! failure is never worse than the previous behaviour.

use std::collections::HashMap;

use image::DynamicImage;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};

use crate::config::MathMode;
use crate::markdown::MathBlockId;

/// Rows the Unicode fallback box costs on top of its formula lines:
/// a top border, a footer line (reason + raw LaTeX), and a bottom border.
///
/// [`entry_height`] adds this to the formula's line count. Getting it wrong is
/// not a cosmetic issue — the block reserves exactly `entry_height` rows, so an
/// undercount silently clips the bottom of the box (the footer was lost this
/// way before the constant existed).
const UNICODE_BOX_CHROME_ROWS: u32 = 3;

/// Minimum height of a math block in display lines, including its chrome.
///
/// One row of content plus [`UNICODE_BOX_CHROME_ROWS`] — the floor at which the
/// fallback box can still show a formula *and* its footer.
pub const MIN_MATH_HEIGHT: u32 = 1 + UNICODE_BOX_CHROME_ROWS;

/// Height reserved for a math block before the cache has any entry for it.
///
/// Deliberately small: most formulas are one or two lines tall, so guessing low
/// keeps the initial layout close to the final one and avoids a visible
/// collapse when the real height arrives.
pub const DEFAULT_MATH_HEIGHT: u32 = 5;

/// Nominal em size handed to RaTeX, in pixels.
///
/// The rasterised image is downscaled to fit the terminal cell grid, so this is
/// a quality knob rather than a size knob: larger values cost more pixels but
/// survive the downscale with cleaner glyph edges. 48 px keeps a typical
/// two-line formula legible at 1× cell density without producing megapixel
/// buffers for a matrix.
const MATH_FONT_SIZE_PX: f32 = 48.0;

/// Padding around the formula, in pixels, so glyphs never touch the edge of the
/// image (and therefore never touch the adjacent text row).
const MATH_PADDING_PX: f32 = 6.0;

/// Reason text shown in the footer when graphics are unavailable inside tmux.
///
/// tmux multiplexes the terminal's output stream and does not reliably pass
/// graphics protocol sequences through, so the image path is skipped there.
const TMUX_DISABLED_REASON: &str = "tmux — graphics disabled";

/// The state of one math block in the render cache.
pub enum MathEntry {
    /// A background render has been spawned; no image yet.
    Pending,
    /// The formula was typeset and encoded for the terminal's graphics protocol.
    ///
    /// Boxed because `StatefulProtocol` is large (>256 bytes) and clippy warns
    /// about enum variants that inflate every instance.
    Ready {
        protocol: Box<StatefulProtocol>,
        /// Height of the rendered image in terminal cells, clamped to
        /// `[MIN_MATH_HEIGHT, math_max_height]`.
        cell_height: u32,
    },
    /// Image rendering was not possible or not wanted; show the Unicode
    /// approximation instead.
    ///
    /// This is the same output the viewer produced before image math existed,
    /// so every failure path degrades to the previous behaviour rather than to
    /// something worse. `reason` explains why in the block footer.
    Unicode {
        /// Pre-converted Unicode text (from `latex_to_unicode`).
        text: String,
        /// Short explanation shown in the footer, e.g. `"parse error: …"`.
        reason: String,
    },
}

/// Parameters for [`MathCache::ensure_queued`].
///
/// Grouped into a struct to stay under clippy's argument limit while keeping
/// call sites readable — same rationale as [`crate::mermaid::MermaidRenderConfig`].
pub struct MathRenderConfig<'a> {
    /// Terminal graphics picker; `None` when graphics are unavailable.
    pub picker: Option<&'a Picker>,
    /// Channel used to deliver completed renders back to the event loop.
    pub action_tx: &'a tokio::sync::mpsc::UnboundedSender<crate::action::Action>,
    /// Whether the process is running inside tmux.
    pub in_tmux: bool,
    /// Foreground colour for glyphs, as sRGB bytes. Taken from the active
    /// theme so the formula matches surrounding text instead of being a black
    /// rectangle on a dark background.
    pub fg_rgb: (u8, u8, u8),
    /// User-configured rendering mode.
    pub mode: MathMode,
    /// User-configured maximum height in display lines.
    pub max_height: u32,
}

/// Per-app cache mapping math block ids to their render state.
pub struct MathCache {
    entries: HashMap<MathBlockId, MathEntry>,
    /// Heights captured the last time each id had a real entry.
    ///
    /// Survives [`MathCache::clear`] so that during a cache refresh (theme
    /// change, mode switch) the height lookup returns the previous value rather
    /// than collapsing to [`DEFAULT_MATH_HEIGHT`] and shifting the document
    /// under the user's cursor.
    last_known_heights: HashMap<MathBlockId, u32>,
}

impl MathCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_known_heights: HashMap::new(),
        }
    }

    /// Return a shared reference to the entry for `id`, if any.
    pub fn get(&self, id: MathBlockId) -> Option<&MathEntry> {
        self.entries.get(&id)
    }

    /// Return a mutable reference to the entry for `id`, if any.
    ///
    /// `StatefulProtocol` needs `&mut` to draw, hence the mutable accessor.
    pub fn get_mut(&mut self, id: MathBlockId) -> Option<&mut MathEntry> {
        self.entries.get_mut(&id)
    }

    /// Insert an entry, overwriting any existing one.
    pub fn insert(&mut self, id: MathBlockId, entry: MathEntry) {
        if let Some(h) = entry_height(&entry) {
            self.last_known_heights.insert(id, h);
        }
        self.entries.insert(id, entry);
    }

    /// Height in display lines that the block for `id` should reserve.
    ///
    /// * `Ready` — the measured image height.
    /// * `Unicode` — the line count of the fallback text plus its border rows.
    /// * `Pending` / absent — the last known height, else a default.
    pub fn height(&self, id: MathBlockId, max_height: u32) -> u32 {
        match self.entries.get(&id) {
            Some(MathEntry::Ready { cell_height, .. }) => *cell_height,
            Some(entry @ MathEntry::Unicode { .. }) => {
                entry_height(entry).unwrap_or(DEFAULT_MATH_HEIGHT)
            }
            Some(MathEntry::Pending) => self
                .last_known_heights
                .get(&id)
                .copied()
                .unwrap_or(DEFAULT_MATH_HEIGHT),
            None => self
                .last_known_heights
                .get(&id)
                .copied()
                .unwrap_or(DEFAULT_MATH_HEIGHT),
        }
        .clamp(MIN_MATH_HEIGHT, max_height.max(MIN_MATH_HEIGHT))
    }

    /// Drop all entries, preserving `last_known_heights`.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Drop every entry whose id is not in `alive`.
    ///
    /// Called after a live reload so formulas that no longer exist in the
    /// document do not pin their images in memory forever.
    pub fn retain(&mut self, alive: &std::collections::HashSet<MathBlockId>) {
        self.entries.retain(|id, _| alive.contains(id));
    }

    /// Ensure `id` has an entry, spawning a background render if appropriate.
    ///
    /// Returns `true` only when a new background task was spawned (the caller
    /// uses this to decide whether a redraw needs to be scheduled).
    ///
    /// # Decision tree
    ///
    /// 1. Already cached → nothing to do.
    /// 2. [`MathMode::Text`] → insert `Unicode`; never touch the image path.
    /// 3. No graphics protocol (or inside tmux) → insert `Unicode` with the
    ///    reason, so the footer explains why the formula is not typeset.
    /// 4. Otherwise → insert `Pending` and spawn the render.
    ///
    /// # Arguments
    ///
    /// * `id`     – stable formula identifier (hash of the LaTeX source).
    /// * `source` – raw LaTeX, without the `$$` delimiters.
    /// * `cfg`    – rendering configuration.
    pub fn ensure_queued(
        &mut self,
        id: MathBlockId,
        source: &str,
        cfg: &MathRenderConfig<'_>,
    ) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }

        // ── Text mode: the historical behaviour, and the default ─────────────
        if cfg.mode == MathMode::Text {
            self.insert(id, unicode_entry(source, "text mode"));
            return false;
        }

        // ── No graphics protocol available ───────────────────────────────────
        let Some(picker) = cfg.picker else {
            let reason = if cfg.in_tmux {
                TMUX_DISABLED_REASON
            } else {
                "graphics unavailable"
            };
            self.insert(id, unicode_entry(source, reason));
            return false;
        };

        self.entries.insert(id, MathEntry::Pending);

        let source = source.to_string();
        let picker = picker.clone();
        let tx = cfg.action_tx.clone();
        let fg_rgb = cfg.fg_rgb;
        let max_height = cfg.max_height;

        // No concurrency cap and no timeout, unlike the mermaid pipeline:
        // RaTeX is a pure layout pass over a parsed AST with no external
        // process and no known hang, and a formula renders in about a
        // millisecond. Capping would add latency for no benefit.
        tokio::task::spawn_blocking(move || {
            let entry = match render_blocking(&source, &picker, fg_rgb, max_height) {
                Ok((protocol, cell_height)) => MathEntry::Ready {
                    protocol: Box::new(protocol),
                    cell_height,
                },
                // Any failure — unparseable LaTeX, a zero-size layout, a PNG
                // that will not decode — falls back to the Unicode rendering
                // the viewer would have shown anyway.
                Err(e) => unicode_entry(&source, &e),
            };
            let _ = tx.send(crate::action::Action::MathReady(id, Box::new(entry)));
        });

        true
    }
}

/// Build a [`MathEntry::Unicode`] from raw LaTeX.
fn unicode_entry(source: &str, reason: &str) -> MathEntry {
    MathEntry::Unicode {
        text: crate::markdown::math::latex_to_unicode(source),
        reason: reason.to_string(),
    }
}

/// Height an entry contributes to `last_known_heights`.
///
/// `Pending` has no measured height yet, so it contributes nothing — that is
/// what makes the `last_known_heights` fallback meaningful.
fn entry_height(entry: &MathEntry) -> Option<u32> {
    match entry {
        MathEntry::Ready { cell_height, .. } => Some(*cell_height),
        MathEntry::Unicode { text, .. } => Some(
            crate::cast::u32_sat(text.lines().count())
                .max(1)
                .saturating_add(UNICODE_BOX_CHROME_ROWS),
        ),
        MathEntry::Pending => None,
    }
}

/// CPU-bound: LaTeX → RaTeX layout → PNG → `DynamicImage` → `StatefulProtocol`.
///
/// Returns the protocol together with the image height in terminal cells.
///
/// # Arguments
///
/// * `source`     – raw LaTeX source, without `$$` delimiters.
/// * `picker`     – terminal graphics picker (supplies the cell pixel size).
/// * `fg_rgb`     – glyph colour, matching the active theme's foreground.
/// * `max_height` – upper bound in display lines.
fn render_blocking(
    source: &str,
    picker: &Picker,
    fg_rgb: (u8, u8, u8),
    max_height: u32,
) -> Result<(StatefulProtocol, u32), String> {
    let png = render_png(source, fg_rgb)?;
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|e| format!("png decode: {e}"))?;
    let cell_height = compute_cell_height(&img, picker, max_height);
    Ok((picker.new_resize_protocol(img), cell_height))
}

/// Typeset `source` and rasterise it to PNG bytes.
///
/// The background is fully transparent and the glyphs are drawn in `fg_rgb`, so
/// the image composites onto whatever the terminal is already showing instead
/// of punching an opaque rectangle into the document.
///
/// Split out from [`render_blocking`] so it can be unit-tested without a
/// terminal graphics picker.
fn render_png(source: &str, fg_rgb: (u8, u8, u8)) -> Result<Vec<u8>, String> {
    use ratex_layout::{LayoutOptions, layout, to_display_list};
    use ratex_parser::parser::parse;
    use ratex_render::{RenderOptions, render_to_png};
    use ratex_types::math_style::MathStyle;

    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("empty formula".to_string());
    }

    let ast = parse(trimmed).map_err(|e| format!("parse error: {}", e.message))?;

    let layout_opts = LayoutOptions::default()
        // Display style is what `$$…$$` means: full-size operators, limits
        // above and below rather than beside.
        .with_style(MathStyle::Display)
        .with_color(to_ratex_color(fg_rgb, 1.0));
    let display_list = to_display_list(&layout(&ast, &layout_opts));

    // A formula that parses but lays out to nothing would produce a 1×1 image
    // and a meaningless block; treat it as a failure so the Unicode fallback
    // takes over.
    if display_list.width <= 0.0 || display_list.height + display_list.depth <= 0.0 {
        return Err("formula has no visible content".to_string());
    }

    let render_opts = RenderOptions {
        font_size: MATH_FONT_SIZE_PX,
        padding: MATH_PADDING_PX,
        background_color: to_ratex_color((0, 0, 0), 0.0),
        // Empty: `embed-fonts` compiles the KaTeX faces into the binary, so
        // there is no directory to look in and nothing for a user to install.
        font_dir: String::new(),
        device_pixel_ratio: 1.0,
    };

    render_to_png(&display_list, &render_opts).map_err(|e| format!("rasterise: {e}"))
}

/// Convert sRGB bytes and an alpha to RaTeX's normalised float colour.
fn to_ratex_color(rgb: (u8, u8, u8), alpha: f32) -> ratex_types::color::Color {
    ratex_types::color::Color {
        r: f32::from(rgb.0) / 255.0,
        g: f32::from(rgb.1) / 255.0,
        b: f32::from(rgb.2) / 255.0,
        a: alpha,
    }
}

/// Natural height of `img` in terminal cells, clamped to
/// `[MIN_MATH_HEIGHT, max_height]`.
fn compute_cell_height(img: &DynamicImage, picker: &Picker, max_height: u32) -> u32 {
    let cell_px_h = picker.font_size().height;
    if cell_px_h == 0 {
        return DEFAULT_MATH_HEIGHT;
    }
    // +1 row so the image never sits flush against the following text line.
    let cells = img
        .height()
        .div_ceil(u32::from(cell_px_h))
        .saturating_add(1);
    cells.clamp(MIN_MATH_HEIGHT, max_height.max(MIN_MATH_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FG: (u8, u8, u8) = (0xE2, 0xE8, 0xF0);

    /// PNG magic number — the first eight bytes of every PNG file.
    const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    /// The formulas from the feature request must rasterise, and each must
    /// produce a *distinct* image — a renderer that returned one constant
    /// buffer would otherwise pass a "did it produce bytes" check.
    #[test]
    fn representative_formulas_render_to_distinct_pngs() {
        let cases = [
            r"\frac{\frac{a}{b}}{c}",
            r"\sum_{i=1}^{n} x_i^{2}",
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
        ];

        let mut seen: Vec<Vec<u8>> = Vec::new();
        for case in cases {
            let png =
                render_png(case, FG).unwrap_or_else(|e| panic!("{case} failed to render: {e}"));
            assert!(
                png.starts_with(PNG_MAGIC),
                "{case} did not produce a PNG (first bytes: {:?})",
                &png[..png.len().min(8)],
            );
            assert!(
                !seen.contains(&png),
                "{case} produced a byte-identical image to an earlier formula — \
                 the renderer is not actually typesetting the input",
            );
            seen.push(png);
        }
    }

    /// Decode a rendered formula's PNG and return its `(width, height)`.
    fn png_dimensions(source: &str) -> Result<(u32, u32), String> {
        let png = render_png(source, FG)?;
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok((img.width(), img.height()))
    }

    /// A nested fraction must be *taller* than a flat one. This is the whole
    /// point of the feature: `latex_to_unicode` renders both on one line, so a
    /// height difference is the evidence that vertical structure survived.
    #[test]
    fn nested_fraction_is_taller_than_flat_fraction() {
        let flat = png_dimensions(r"a + b").expect("flat formula must render");
        let one_level = png_dimensions(r"\frac{a}{b}").expect("fraction must render");
        let two_level = png_dimensions(r"\frac{\frac{a}{b}}{c}").expect("nested must render");

        assert!(
            one_level.1 > flat.1,
            "a fraction must be taller than inline text: {} vs {}",
            one_level.1,
            flat.1,
        );
        assert!(
            two_level.1 > one_level.1,
            "a nested fraction must be taller than a single fraction: {} vs {}",
            two_level.1,
            one_level.1,
        );
    }

    /// Unparseable LaTeX must return `Err`, not panic and not produce a
    /// misleading image. The message is surfaced in the block footer, so it
    /// also has to name the offending command.
    #[test]
    fn invalid_latex_returns_a_descriptive_error() {
        let err = render_png(r"\notacommand{x}", FG).expect_err("must not render");
        assert!(
            err.contains("parse error"),
            "error should be tagged as a parse failure: {err}"
        );
        assert!(
            err.contains("notacommand"),
            "error should name the unknown command so the footer is useful: {err}"
        );
    }

    /// Empty and whitespace-only formulas are rejected before they reach the
    /// rasteriser, where they would produce a degenerate 1×1 image.
    #[test]
    fn empty_formula_is_rejected() {
        for source in ["", "   ", "\n\t "] {
            let err = render_png(source, FG).expect_err("empty input must not render");
            assert!(
                err.contains("empty"),
                "expected an 'empty formula' error for {source:?}, got: {err}"
            );
        }
    }

    /// The glyph colour must actually reach the pixels. Rendering the same
    /// formula in two different colours has to produce different bytes —
    /// otherwise theme-matching is silently a no-op.
    #[test]
    fn foreground_colour_changes_the_rendered_pixels() {
        let light = render_png(r"x^2", (0xE2, 0xE8, 0xF0)).expect("render");
        let dark = render_png(r"x^2", (0x11, 0x11, 0x11)).expect("render");
        assert_ne!(
            light, dark,
            "foreground colour had no effect on the rasterised output",
        );
    }

    /// The image is composited over terminal content, so the background must
    /// be fully transparent rather than an opaque rectangle.
    #[test]
    fn background_is_transparent() {
        let png = render_png(r"x^2", FG).expect("render");
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("decode")
            .to_rgba8();

        // The top-left pixel is padding, which no glyph can reach.
        assert_eq!(
            img.get_pixel(0, 0).0[3],
            0,
            "padding pixel must be fully transparent",
        );
    }

    /// `MathCache::height` must never report a height that would make the
    /// block unusable, whatever the configured maximum.
    #[test]
    fn height_is_clamped_to_a_usable_range() {
        let mut cache = MathCache::new();
        let id = MathBlockId(1);
        cache.insert(
            id,
            MathEntry::Unicode {
                text: "a\nb\nc\nd\ne\nf\ng\nh".to_string(),
                reason: "test".to_string(),
            },
        );

        // A pathologically small max must still leave room for the chrome.
        assert_eq!(cache.height(id, 0), MIN_MATH_HEIGHT);
        // A generous max lets the real height through: 8 formula lines plus
        // the top border, footer, and bottom border.
        assert_eq!(cache.height(id, 50), 8 + UNICODE_BOX_CHROME_ROWS);
        // The max is an upper bound.
        assert_eq!(cache.height(id, 6), 6);
    }

    /// The reserved height must fit everything the fallback box draws: the
    /// formula lines, the footer that carries the reason and the raw LaTeX,
    /// and both borders.
    ///
    /// This is the bug the constant exists for — a one-line formula reserved
    /// only 3 rows, which is a bordered box with a single inner row, so the
    /// footer was silently clipped and the user never learned *why* their
    /// formula had not been typeset.
    #[test]
    fn unicode_height_leaves_room_for_the_footer_and_both_borders() {
        for (formula_lines, text) in [(1usize, "a/b"), (3, "l1\nl2\nl3")] {
            let entry = MathEntry::Unicode {
                text: text.to_string(),
                reason: "parse error".to_string(),
            };
            let reserved = entry_height(&entry).expect("Unicode entries have a height");

            // What `math_draw::build_unicode_text` produces: one line per
            // formula row plus exactly one footer.
            let drawn_lines = formula_lines + 1;
            // Plus the two border rows the surrounding Block occupies.
            let required = crate::cast::u32_sat(drawn_lines) + 2;

            assert!(
                reserved >= required,
                "{formula_lines}-line formula reserved {reserved} rows but needs {required} \
                 (formula + footer + 2 borders) — the footer would be clipped",
            );
        }
    }

    /// While a re-render is in flight the block must keep its previous height,
    /// or the document would collapse and drag the cursor with it.
    #[test]
    fn pending_reuses_the_last_known_height() {
        let mut cache = MathCache::new();
        let id = MathBlockId(7);

        cache.insert(
            id,
            MathEntry::Ready {
                protocol: Box::new(dummy_protocol()),
                cell_height: 12,
            },
        );
        assert_eq!(cache.height(id, 50), 12);

        // A theme change clears entries but must not forget the height.
        cache.clear();
        assert_eq!(cache.height(id, 50), 12, "cleared cache lost the height");

        cache.insert(id, MathEntry::Pending);
        assert_eq!(cache.height(id, 50), 12, "pending entry lost the height");
    }

    /// `retain` must drop formulas that disappeared from the document and keep
    /// the ones that survived.
    #[test]
    fn retain_drops_only_absent_ids() {
        let mut cache = MathCache::new();
        let kept = MathBlockId(1);
        let dropped = MathBlockId(2);
        cache.insert(kept, unicode_entry("x", "t"));
        cache.insert(dropped, unicode_entry("y", "t"));

        let alive: std::collections::HashSet<MathBlockId> = std::iter::once(kept).collect();
        cache.retain(&alive);

        assert!(cache.get(kept).is_some(), "surviving formula was dropped");
        assert!(cache.get(dropped).is_none(), "stale formula was retained");
    }

    /// End-to-end through the CPU path a background task runs: LaTeX in, a
    /// terminal-ready protocol and a sane cell height out.
    ///
    /// This is the one test that exercises PNG decode and protocol
    /// construction together — the unit tests above stop at the PNG bytes.
    #[test]
    fn render_blocking_produces_a_protocol_and_bounded_height() {
        let picker = test_picker();

        let (_protocol, cell_height) =
            render_blocking(r"\frac{\frac{a}{b}}{c}", &picker, FG, 20).expect("must render");

        assert!(
            (MIN_MATH_HEIGHT..=20).contains(&cell_height),
            "cell height {cell_height} outside [{MIN_MATH_HEIGHT}, 20]",
        );

        // A taller formula must reserve more rows than a short one — proof the
        // height is measured from the image rather than being a constant.
        let (_, short) = render_blocking(r"x", &picker, FG, 40).expect("must render");
        let (_, tall) = render_blocking(
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            &picker,
            FG,
            40,
        )
        .expect("must render");
        assert!(
            tall > short,
            "a matrix ({tall} rows) must reserve more than a single glyph ({short} rows)",
        );
    }

    /// `max_height` is a hard ceiling: a formula that rasterises taller than
    /// the configured maximum must be clamped, not allowed to swallow the
    /// viewport.
    #[test]
    fn cell_height_respects_the_configured_maximum() {
        let picker = test_picker();
        let (_, height) = render_blocking(
            r"\frac{\frac{\frac{a}{b}}{c}}{\frac{d}{e}}",
            &picker,
            FG,
            MIN_MATH_HEIGHT,
        )
        .expect("must render");
        assert_eq!(
            height, MIN_MATH_HEIGHT,
            "height was not clamped to the maximum"
        );
    }

    /// A picker reporting a zero-pixel cell must not divide by zero.
    #[test]
    fn zero_font_size_falls_back_to_the_default_height() {
        #[allow(deprecated)]
        let picker = Picker::from_fontsize(ratatui_image::FontSize {
            width: 0,
            height: 0,
        });
        let img = DynamicImage::new_rgba8(100, 100);
        assert_eq!(compute_cell_height(&img, &picker, 20), DEFAULT_MATH_HEIGHT);
    }

    /// A picker with a realistic 8x16 cell, usable without querying a terminal.
    fn test_picker() -> Picker {
        #[allow(deprecated)]
        Picker::from_fontsize(ratatui_image::FontSize {
            width: 8,
            height: 16,
        })
    }

    /// Build a throwaway `StatefulProtocol` for height bookkeeping tests.
    fn dummy_protocol() -> StatefulProtocol {
        test_picker().new_resize_protocol(DynamicImage::new_rgba8(8, 16))
    }
}
