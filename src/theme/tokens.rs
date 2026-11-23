//! Semantic design tokens. The source of truth for every theme; the
//! flat [`super::Palette`] is a derived view (`From<Tokens>`).
//!
//! Tokens are nested into per-purpose sub-structs (`Surface`, `Text`,
//! `State`, …) so a contributor reading `state.selection_bg` knows
//! immediately it's an interaction state, not a brand colour. Each
//! sub-struct is `Copy` and trivially small (a handful of `Color`s),
//! so passing `Tokens` by value is essentially free.
//!
//! Theme bodies live in [`super::themes`] — one `fn` per theme,
//! returning a fully-derived `Tokens`. Add new themes there, not here.

use super::Theme;
use ratatui::style::Color;

/// Top-level design-token bag. One per theme.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub surface: Surface,
    pub text: Text,
    pub state: State,
    pub accent: Accent,
    pub syntax: Syntax,
    pub heading: Heading,
    pub status: Status,
    pub list: List,
    pub table: Table,
    pub git: Git,
}

/// Surface tiers — backgrounds the user sees through the layout.
#[derive(Debug, Clone, Copy)]
pub struct Surface {
    /// Page-level background.
    pub base: Color,
    /// Slightly elevated surface (code blocks, popups, status bar).
    pub raised: Color,
    /// Default border line color for unfocused panels.
    pub border: Color,
}

/// Reading-text colors that overlay the surface tiers.
#[derive(Debug, Clone, Copy)]
pub struct Text {
    pub primary: Color,
    pub muted: Color,
    /// Foreground for text drawn on an `accent.primary`-coloured background.
    pub on_accent: Color,
    /// Bold widget titles.
    pub title: Color,
}

/// Interaction-state backgrounds and the foregrounds that overlay them.
#[derive(Debug, Clone, Copy)]
pub struct State {
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// Border colour for the focused panel and the "currently here" cue.
    pub focus: Color,
    pub search_bg: Color,
    pub current_match_bg: Color,
    /// Foreground used on both `search_bg` and `current_match_bg`.
    pub match_fg: Color,
}

/// Brand / emphasis hues used for interactive accents and links.
#[derive(Debug, Clone, Copy)]
pub struct Accent {
    pub primary: Color,
    pub alt: Color,
    pub link: Color,
}

/// Syntax-highlighting and code-block colors.
///
/// Code-block *background* lives on `surface.raised` — semantically a
/// code block is a raised surface tier, and pinning the colour there
/// keeps every theme's code/status/help backgrounds in sync without
/// repeating the value across slots.
#[derive(Debug, Clone, Copy)]
pub struct Syntax {
    pub inline_code: Color,
    pub code_fg: Color,
    pub code_border: Color,
}

/// Heading hierarchy colors.
#[derive(Debug, Clone, Copy)]
pub struct Heading {
    pub h1: Color,
    pub h2: Color,
    pub h3: Color,
    /// h4–h6 and any other headings beyond the styled tier.
    pub other: Color,
}

/// Status bar, help overlay, and gutter colors.
#[derive(Debug, Clone, Copy)]
pub struct Status {
    pub bg: Color,
    pub fg: Color,
    pub help_bg: Color,
    pub gutter: Color,
}

/// List markers and block-quote chrome.
#[derive(Debug, Clone, Copy)]
pub struct List {
    pub marker: Color,
    pub task_marker: Color,
    pub block_quote_fg: Color,
    pub block_quote_border: Color,
}

/// Table chrome.
#[derive(Debug, Clone, Copy)]
pub struct Table {
    pub header: Color,
    pub border: Color,
}

/// Git-status decorations in the file tree.
#[derive(Debug, Clone, Copy)]
pub struct Git {
    pub new: Color,
    pub modified: Color,
}

impl Tokens {
    /// Construct the design tokens for the given theme. Dispatches to
    /// the per-theme function in [`super::themes`].
    #[must_use]
    pub fn from_theme(theme: Theme) -> Self {
        match theme {
            Theme::Default => super::themes::default_(),
            Theme::Dracula => super::themes::dracula(),
            Theme::SolarizedDark => super::themes::solarized_dark(),
            Theme::SolarizedLight => super::themes::solarized_light(),
            Theme::Nord => super::themes::nord(),
            Theme::GruvboxDark => super::themes::gruvbox_dark(),
            Theme::GruvboxLight => super::themes::gruvbox_light(),
            Theme::GithubLight => super::themes::github_light(),
            Theme::GaiaAether => super::themes::gaia_aether(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::contrast::{contrast_ratio, luminance_delta};
    use super::*;

    /// Minimum luminance separation between selection_bg and its surfaces
    /// (scaled 0-100 — see `luminance_delta` doc). Empirically 3.0 catches
    /// the original Solarized Light bug (`selection_bg == code_bg`,
    /// delta = 0) while letting a 10% lighten/darken on bright surfaces
    /// pass cleanly.
    const MIN_SELECTION_DELTA: f64 = 3.0;

    /// Derived selection backgrounds must be perceptually distinct from
    /// both surface tiers. This is the test that would have caught the
    /// 2026-04-24 Solarized Light bug *automatically* — even if a future
    /// contributor sets `state.selection_bg = surface.raised` by hand,
    /// the test fails before the change ever ships.
    #[test]
    fn selection_bg_is_distinct_from_surfaces() {
        let mut failures: Vec<String> = Vec::new();
        for &theme in Theme::ALL {
            let t = Tokens::from_theme(theme);
            for (label, surface) in [
                ("surface.base", t.surface.base),
                // `surface.raised` is also where `palette.code_bg` sources
                // from, so this single check covers both surfaces and the
                // code-block background in one go.
                ("surface.raised", t.surface.raised),
            ] {
                if let Some(delta) = luminance_delta(t.state.selection_bg, surface)
                    && delta < MIN_SELECTION_DELTA
                {
                    failures.push(format!(
                        "  {theme:?}: selection_bg too close to {label} (Δ={delta:.2}, min={MIN_SELECTION_DELTA})",
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "derived selection backgrounds collide perceptually with surfaces:\n{}",
            failures.join("\n"),
        );
    }

    /// Focus state must be visible against the page background. Relaxed
    /// from text-AA (4.5:1) to UI-component contrast (3.0:1, WCAG SC
    /// 1.4.11) because focus is a decoration line, not text.
    #[test]
    fn focus_is_visible_against_surface() {
        const MIN_FOCUS_RATIO: f64 = 3.0;
        let mut failures: Vec<String> = Vec::new();
        for &theme in Theme::ALL {
            let t = Tokens::from_theme(theme);
            if let Some(ratio) = contrast_ratio(t.state.focus, t.surface.base)
                && ratio < MIN_FOCUS_RATIO
            {
                failures.push(format!(
                    "  {theme:?}: focus on surface.base = {ratio:.2}:1 < {MIN_FOCUS_RATIO}:1",
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "focus state too low contrast:\n{}",
            failures.join("\n")
        );
    }
}
