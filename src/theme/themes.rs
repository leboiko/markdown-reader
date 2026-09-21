//! Per-theme `Tokens` constructors. One `fn` per `Theme` variant.
//!
//! Conventions used here:
//!
//! * Each theme starts with its **base hues** (the colours the theme's
//!   designer chose). Everything else either reuses a base hue or is
//!   derived from one via [`super::contrast::ColorOps`].
//! * Where a theme's source palette already has a name for a slot
//!   (Solarized's `base02`, Gruvbox's `bg2`, Nord's `nord11`), the
//!   comment carries it through. Future contributors looking to tweak
//!   a theme can find the source-palette name in one place.
//! * Slots that re-use a base hue (e.g. `state.focus = accent.primary`,
//!   `accent.link = accent.primary`) are written as the assignment to
//!   make the relationship explicit. Pure passthrough slots (no
//!   derivation, no relationship) just take the literal `Color`.
//! * The `code as art` bias: prefer **named base hues + assignments**
//!   over a wall of RGB literals. Where a theme's design doesn't allow
//!   simple reuse (Default has many `Color::Cyan` / named-ANSI slots),
//!   keep the explicit values — readability beats forced derivation.
//! * **Code-block background** is sourced from `surface.raised` in the
//!   `From<Tokens> for Palette` mapping — themes don't set it
//!   separately. See [`super::tokens::Syntax`] for rationale.

#![allow(clippy::too_many_lines)]

use super::tokens::*;
use ratatui::style::Color;

/// Default theme — neutral dark background, ANSI named accents. The
/// only theme that uses named (non-RGB) colors heavily, so most slots
/// stay explicit.
pub(super) fn default_() -> Tokens {
    let bg = Color::Rgb(20, 20, 30);
    let raised = Color::Rgb(40, 40, 40);
    let cyan = Color::Cyan;
    let yellow = Color::Yellow;
    Tokens {
        surface: Surface {
            base: bg,
            raised,
            border: Color::DarkGray,
        },
        text: Text {
            primary: Color::Rgb(220, 220, 220),
            muted: Color::DarkGray,
            on_accent: Color::Black,
            title: Color::Rgb(220, 220, 220),
        },
        state: State {
            selection_bg: Color::Rgb(0, 160, 80),
            selection_fg: Color::Black,
            focus: cyan,
            search_bg: yellow,
            current_match_bg: Color::Rgb(255, 120, 0),
            match_fg: Color::Black,
        },
        accent: Accent {
            primary: cyan,
            alt: yellow,
            link: Color::Blue,
        },
        syntax: Syntax {
            inline_code: Color::Green,
            code_fg: Color::Rgb(180, 200, 180),
            code_border: Color::DarkGray,
        },
        heading: Heading {
            h1: cyan,
            h2: Color::Blue,
            h3: Color::Magenta,
            other: Color::White,
        },
        status: Status {
            // Default uses a lower-elevation status bar than `raised`,
            // making explicit rather than `surface.raised`. The design
            // intent is a clear three-tier hierarchy: base < status < raised.
            bg: Color::Rgb(30, 30, 30),
            fg: Color::Gray,
            help_bg: bg,
            gutter: Color::DarkGray,
        },
        list: List {
            marker: yellow,
            task_marker: cyan,
            block_quote_fg: Color::Gray,
            block_quote_border: Color::DarkGray,
        },
        table: Table {
            header: cyan,
            border: Color::DarkGray,
        },
        git: Git {
            new: Color::Rgb(80, 200, 120),
            modified: Color::Rgb(220, 180, 60),
        },
    }
}

/// Dracula — official palette: <https://draculatheme.com/contribute>
pub(super) fn dracula() -> Tokens {
    let bg = Color::Rgb(40, 42, 54);
    let fg = Color::Rgb(248, 248, 242);
    let comment = Color::Rgb(98, 114, 164);
    let current_line = Color::Rgb(68, 71, 90);
    let purple = Color::Rgb(189, 147, 249);
    let pink = Color::Rgb(255, 121, 198);
    let green = Color::Rgb(80, 250, 123);
    let yellow = Color::Rgb(241, 250, 140);
    let cyan = Color::Rgb(139, 233, 253);
    Tokens {
        surface: Surface {
            base: bg,
            raised: bg, // Dracula has only one bg tier; code blocks share it.
            border: current_line,
        },
        text: Text {
            primary: fg,
            muted: comment,
            // Dark text on bright purple: 1.23.0 audit fix (was fg, 2.26:1).
            on_accent: bg,
            title: fg,
        },
        state: State {
            selection_bg: current_line,
            selection_fg: fg,
            focus: purple,
            search_bg: yellow,
            current_match_bg: pink,
            match_fg: bg,
        },
        accent: Accent {
            primary: purple,
            alt: yellow,
            link: cyan,
        },
        syntax: Syntax {
            inline_code: green,
            code_fg: fg,
            code_border: comment,
        },
        heading: Heading {
            h1: pink,
            h2: purple,
            h3: green,
            other: fg,
        },
        status: Status {
            bg,
            // 1.23.0 audit fix: was comment (3.03:1), bumped to foreground.
            fg,
            help_bg: bg,
            gutter: comment,
        },
        list: List {
            marker: yellow,
            task_marker: green,
            block_quote_fg: comment,
            block_quote_border: comment,
        },
        table: Table {
            header: pink,
            border: comment,
        },
        git: Git {
            new: green,
            modified: yellow,
        },
    }
}

/// Solarized Dark — Ethan Schoonover: <https://ethanschoonover.com/solarized/>
pub(super) fn solarized_dark() -> Tokens {
    let base03 = Color::Rgb(0, 43, 54);
    let base02 = Color::Rgb(7, 54, 66);
    let base01 = Color::Rgb(88, 110, 117);
    let base0 = Color::Rgb(131, 148, 150);
    let base1 = Color::Rgb(147, 161, 161);
    let base3 = Color::Rgb(253, 246, 227);
    let yellow = Color::Rgb(181, 137, 0);
    let orange = Color::Rgb(203, 75, 22);
    let blue = Color::Rgb(38, 139, 210);
    let cyan = Color::Rgb(42, 161, 152);
    let green = Color::Rgb(133, 153, 0);
    Tokens {
        surface: Surface {
            base: base03,
            raised: base02,
            border: base01,
        },
        text: Text {
            primary: base0,
            muted: base01,
            // 1.23.0 audit fix: base1 on blue was 1.38:1; black ~6:1.
            on_accent: Color::Rgb(0, 0, 0),
            title: base1,
        },
        state: State {
            // 1.23.0 audit fix: was base02 (= surface.raised → invisible).
            selection_bg: base01,
            selection_fg: base3,
            focus: blue,
            search_bg: yellow,
            current_match_bg: orange,
            // 1.23.0 audit fix: base03 on orange was 3.26:1.
            match_fg: Color::Rgb(0, 0, 0),
        },
        accent: Accent {
            primary: blue,
            alt: yellow,
            link: blue,
        },
        syntax: Syntax {
            inline_code: green,
            // 1.23.0 audit fix: base0 on base02 was 4.11:1; bumped to base1.
            code_fg: base1,
            code_border: base01,
        },
        heading: Heading {
            h1: orange,
            h2: blue,
            h3: cyan,
            other: base0,
        },
        status: Status {
            bg: base02,
            // 1.23.0 audit fix: base01 on base02 was 2.42:1; bumped to base1.
            fg: base1,
            help_bg: base02,
            gutter: base01,
        },
        list: List {
            marker: yellow,
            task_marker: cyan,
            block_quote_fg: base01,
            block_quote_border: base01,
        },
        table: Table {
            header: orange,
            border: base01,
        },
        git: Git {
            new: green,
            modified: yellow,
        },
    }
}

/// Solarized Light — Ethan Schoonover: <https://ethanschoonover.com/solarized/>
///
/// Note: Solarized intentionally ships sub-AA contrast for a "soft"
/// reading look (base00 on base3 ≈ 4.13:1). For a markdown reader where
/// users actually parse text, primary text is bumped to base02 so
/// reading and code blocks both reach AA. Accent slots stay canonical.
pub(super) fn solarized_light() -> Tokens {
    let base02 = Color::Rgb(7, 54, 66);
    let base00 = Color::Rgb(101, 123, 131);
    let base1 = Color::Rgb(147, 161, 161);
    let base2 = Color::Rgb(238, 232, 213);
    let base3 = Color::Rgb(253, 246, 227);
    let yellow = Color::Rgb(181, 137, 0);
    let orange = Color::Rgb(203, 75, 22);
    let blue = Color::Rgb(38, 139, 210);
    let cyan = Color::Rgb(42, 161, 152);
    let green = Color::Rgb(133, 153, 0);
    Tokens {
        surface: Surface {
            base: base3,
            raised: base2,
            border: base2,
        },
        text: Text {
            primary: base02, // 1.23.0 audit fix: was base00, sub-AA.
            muted: base00,   // was base1.
            // 1.23.0 audit fix: base3 on blue was 3.41:1; black ~5:1.
            on_accent: Color::Rgb(0, 0, 0),
            title: base02,
        },
        state: State {
            // 1.23.0 audit fix: was base2 (= surface.raised → invisible).
            selection_bg: base1,
            selection_fg: base02,
            focus: blue,
            search_bg: yellow,
            current_match_bg: orange,
            // 1.23.0 audit fix: base3 on yellow/orange both sub-AA.
            match_fg: Color::Rgb(0, 0, 0),
        },
        accent: Accent {
            primary: blue,
            alt: yellow,
            link: blue,
        },
        syntax: Syntax {
            inline_code: green,
            code_fg: base02, // 1.23.0 audit fix: was base00, 3.64:1.
            code_border: base1,
        },
        heading: Heading {
            h1: orange,
            h2: blue,
            h3: cyan,
            other: base02,
        },
        status: Status {
            bg: base2,
            fg: base02, // 1.23.0 audit fix: was base00, 3.64:1.
            help_bg: base2,
            gutter: base1,
        },
        list: List {
            marker: yellow,
            task_marker: cyan,
            block_quote_fg: base00,
            block_quote_border: base1,
        },
        table: Table {
            header: orange,
            border: base1,
        },
        git: Git {
            new: green,
            modified: yellow,
        },
    }
}

/// Nord — Arctic palette: <https://www.nordtheme.com/docs/colors-and-palettes>
///
/// Polar Night gradient (canonical):
///   nord0 = #2e3440  base background
///   nord1 = #3b4252  raised surface tier
///   nord2 = #434c5e  selection / current-line
///   nord3 = #4c566a  borders / muted text
pub(super) fn nord() -> Tokens {
    let nord0 = Color::Rgb(46, 52, 64);
    let nord1 = Color::Rgb(59, 66, 82);
    // nord2 = (67, 76, 94) — currently no slot uses it; the gradient
    // goes nord0 → nord1 → nord3 (skipping nord2 to keep selection
    // perceptually distinct from raised).
    let nord3 = Color::Rgb(76, 86, 106);
    let nord4 = Color::Rgb(216, 222, 233);
    let nord6 = Color::Rgb(236, 239, 244);
    let nord8 = Color::Rgb(136, 192, 208);
    let nord9 = Color::Rgb(129, 161, 193);
    let nord10 = Color::Rgb(94, 129, 172);
    let nord11 = Color::Rgb(191, 97, 106);
    let nord13 = Color::Rgb(235, 203, 139);
    let nord14 = Color::Rgb(163, 190, 140);
    Tokens {
        surface: Surface {
            base: nord0,
            raised: nord1,
            border: nord3,
        },
        text: Text {
            primary: nord4,
            muted: nord3,
            on_accent: nord0,
            title: nord6,
        },
        state: State {
            // nord3 is the most-elevated Polar Night tier — gives
            // selection a clear two-step lift from `raised` (nord1)
            // while staying canonical. Adjacent tiers (nord1→nord2)
            // measured Δ=1.7 — perceptually too close.
            selection_bg: nord3,
            selection_fg: nord6,
            focus: nord8,
            search_bg: nord13,
            current_match_bg: nord11,
            // 1.23.0 audit fix: nord0 on nord11 was 3.05:1.
            match_fg: Color::Rgb(0, 0, 0),
        },
        accent: Accent {
            primary: nord8,
            alt: nord13,
            link: nord9,
        },
        syntax: Syntax {
            inline_code: nord14,
            code_fg: nord4,
            code_border: nord3,
        },
        heading: Heading {
            h1: nord11,
            h2: nord8,
            h3: nord14,
            other: nord4,
        },
        status: Status {
            bg: nord1,
            // 1.23.0 audit fix: was nord3 on nord1 = 1.36:1, basically illegible.
            fg: nord4,
            help_bg: nord1,
            gutter: nord3,
        },
        list: List {
            marker: nord13,
            task_marker: nord14,
            block_quote_fg: nord3,
            block_quote_border: nord3,
        },
        table: Table {
            header: nord10,
            border: nord3,
        },
        git: Git {
            new: nord14,
            modified: nord13,
        },
    }
}

/// Gruvbox Dark — <https://github.com/morhetz/gruvbox>
pub(super) fn gruvbox_dark() -> Tokens {
    let bg = Color::Rgb(40, 40, 40);
    let bg1 = Color::Rgb(50, 48, 47);
    let bg3 = Color::Rgb(80, 73, 69);
    let bg4 = Color::Rgb(102, 92, 84);
    let fg = Color::Rgb(235, 219, 178);
    let gray = Color::Rgb(146, 131, 116);
    let red = Color::Rgb(251, 73, 52);
    let orange = Color::Rgb(214, 93, 14);
    let yellow = Color::Rgb(250, 189, 47);
    let green = Color::Rgb(184, 187, 38);
    let aqua = Color::Rgb(131, 165, 152);
    Tokens {
        surface: Surface {
            base: bg,
            raised: bg1,
            border: bg3,
        },
        text: Text {
            primary: fg,
            muted: gray,
            on_accent: bg,
            title: fg,
        },
        state: State {
            selection_bg: bg3,
            selection_fg: fg,
            focus: orange,
            search_bg: yellow,
            current_match_bg: red,
            // 1.23.0 audit fix: bg on red was 4.29:1, just under AA.
            match_fg: Color::Rgb(0, 0, 0),
        },
        accent: Accent {
            primary: yellow,
            alt: green,
            link: aqua,
        },
        syntax: Syntax {
            inline_code: green,
            code_fg: fg,
            code_border: bg3,
        },
        heading: Heading {
            h1: red,
            h2: yellow,
            h3: green,
            other: fg,
        },
        status: Status {
            bg: bg1,
            // 1.23.0 audit fix: was gray (3.58:1); bumped to fg.
            fg,
            help_bg: bg1,
            gutter: bg4,
        },
        list: List {
            marker: yellow,
            task_marker: green,
            block_quote_fg: gray,
            block_quote_border: gray,
        },
        table: Table {
            header: orange,
            border: bg3,
        },
        git: Git {
            new: green,
            modified: yellow,
        },
    }
}

/// Gruvbox Light — <https://github.com/morhetz/gruvbox>
pub(super) fn gruvbox_light() -> Tokens {
    let bg = Color::Rgb(251, 241, 199);
    let bg1 = Color::Rgb(235, 219, 178);
    let bg2 = Color::Rgb(213, 196, 161);
    let fg = Color::Rgb(60, 56, 54);
    let fg1 = Color::Rgb(80, 73, 69);
    let gray = Color::Rgb(146, 131, 116);
    let red = Color::Rgb(204, 36, 29);
    let orange = Color::Rgb(214, 93, 14);
    let yellow = Color::Rgb(215, 153, 33);
    let green = Color::Rgb(152, 151, 26);
    let aqua = Color::Rgb(104, 157, 106);
    let blue = Color::Rgb(69, 133, 136);
    let purple = Color::Rgb(177, 98, 134);
    Tokens {
        surface: Surface {
            base: bg,
            raised: bg1,
            border: bg2,
        },
        text: Text {
            primary: fg,
            muted: gray,
            on_accent: fg,
            title: fg,
        },
        state: State {
            // 1.23.0 audit fix: was bg1 (= surface.raised → invisible).
            selection_bg: bg2,
            selection_fg: fg,
            focus: orange,
            search_bg: yellow,
            current_match_bg: orange,
            // 1.23.0 audit fix: bg on yellow/orange both sub-AA.
            match_fg: Color::Rgb(0, 0, 0),
        },
        accent: Accent {
            primary: yellow,
            alt: green,
            link: blue,
        },
        syntax: Syntax {
            inline_code: purple,
            code_fg: fg,
            code_border: bg2,
        },
        heading: Heading {
            h1: red,
            h2: yellow,
            h3: green,
            other: fg,
        },
        status: Status {
            bg: bg1,
            fg: fg1,
            help_bg: bg1,
            gutter: gray,
        },
        list: List {
            marker: yellow,
            task_marker: aqua,
            block_quote_fg: gray,
            block_quote_border: bg2,
        },
        table: Table {
            header: orange,
            border: bg2,
        },
        git: Git {
            new: green,
            modified: yellow,
        },
    }
}

/// GitHub Light — Primer Primitives: <https://primer.style/primitives/colors>
pub(super) fn github_light() -> Tokens {
    let canvas = Color::Rgb(255, 255, 255);
    let subtle = Color::Rgb(246, 248, 250);
    let border = Color::Rgb(208, 215, 222);
    let fg = Color::Rgb(31, 35, 40);
    let muted = Color::Rgb(101, 109, 118);
    let blue = Color::Rgb(9, 105, 218); // accent.fg
    let amber = Color::Rgb(154, 103, 0); // attention.fg
    let green = Color::Rgb(26, 127, 55); // success.fg
    let red = Color::Rgb(207, 34, 46); // danger.fg
    let amber_emph = Color::Rgb(255, 211, 61); // attention.emphasis
    let severe_emph = Color::Rgb(255, 143, 0); // severe.emphasis
    let accent_subtle = Color::Rgb(221, 244, 255);
    Tokens {
        surface: Surface {
            base: canvas,
            raised: subtle,
            border,
        },
        text: Text {
            primary: fg,
            muted,
            // White on the vivid blue — `selection_fg` is also #0969da which
            // would produce invisible blue-on-blue text if used on accent.
            on_accent: canvas,
            title: fg,
        },
        state: State {
            selection_bg: accent_subtle,
            selection_fg: blue,
            focus: blue,
            search_bg: amber_emph,
            current_match_bg: severe_emph,
            match_fg: fg,
        },
        accent: Accent {
            primary: blue,
            alt: amber,
            link: blue,
        },
        syntax: Syntax {
            inline_code: red,
            code_fg: fg,
            code_border: border,
        },
        heading: Heading {
            h1: blue,
            h2: amber,
            h3: green,
            other: fg,
        },
        status: Status {
            bg: subtle,
            fg: muted,
            help_bg: subtle,
            gutter: muted,
        },
        list: List {
            marker: amber,
            task_marker: green,
            block_quote_fg: muted,
            block_quote_border: border,
        },
        table: Table {
            header: blue,
            border,
        },
        git: Git {
            new: green,
            modified: amber,
        },
    }
}

/// Gaia Aether — the dark Gruvbox/terracotta palette used across the author's
/// desktop theme ("AETHER / G.A.I.A v2": Gruvbox, terracotta, and dusty mauve).
///
/// Slots carry the source-palette names from that theme's `colors.toml`:
/// `background` `#181c22`, `lighter_background` `#1e2228`, `foreground`
/// `#f2e5bc`, `dark_foreground` `#8a7e70`, `accent` `#a0607a`, `selection`
/// `#261a28`, plus the Gruvbox-derived hue slots.
///
/// Three slots deviate from the source palette because its literal values
/// fail this crate's own contrast audit:
///
/// * `accent.primary` is the theme's dusty-mauve/tan `magenta` `#c98fa0`
///   rather than `accent` `#a0607a`. No foreground reaches WCAG AA (4.5:1)
///   on `#a0607a` — its ceiling is 3.76:1 — so it cannot carry the
///   `on_accent` text drawn by the status bar and tab bar. `#a0607a` stays
///   in the palette as `accent.alt`.
/// * `state.selection_bg` `#503a4a` is a lightened `selection` — the
///   source `#261a28` sits only 0.16 luminance units from `background`,
///   below the `MIN_SELECTION_DELTA` of 3.0, and would render invisible.
/// * `state.focus` is the magenta accent, 6.45:1 on `background`.
pub(super) fn gaia_aether() -> Tokens {
    // AETHER / G.A.I.A v2 base surfaces and reading text.
    let bg = Color::Rgb(24, 28, 34); // background        #181c22
    let raised = Color::Rgb(30, 34, 40); // lighter_background #1e2228
    let brown = Color::Rgb(60, 56, 54); // brown             #3c3836
    let fg = Color::Rgb(242, 229, 188); // foreground        #f2e5bc
    let gray = Color::Rgb(168, 153, 132); // gruvbox fg4       #a89984
    let dark_fg = Color::Rgb(138, 126, 112); // dark_foreground   #8a7e70
    // Hue slots, Gruvbox-derived in the source theme.
    let orange = Color::Rgb(208, 128, 80); // orange            #d08050
    let yellow = Color::Rgb(184, 151, 90); // yellow            #b8975a
    let bright_yellow = Color::Rgb(212, 176, 122); // bright_yellow     #d4b07a
    let green = Color::Rgb(122, 158, 126); // green             #7a9e7e
    let cyan = Color::Rgb(131, 165, 152); // cyan              #83a598
    let mauve = Color::Rgb(201, 143, 160); // magenta           #c98fa0
    let deep_mauve = Color::Rgb(160, 96, 122); // accent            #a0607a
    let selection = Color::Rgb(80, 58, 74); // selection, lightened #261a28
    Tokens {
        surface: Surface {
            base: bg,
            raised,
            border: brown,
        },
        text: Text {
            primary: fg,
            muted: gray,
            // Dark base on the bright mauve accent: 6.45:1.
            on_accent: bg,
            title: fg,
        },
        state: State {
            selection_bg: selection,
            selection_fg: fg,
            focus: mauve,
            search_bg: bright_yellow,
            current_match_bg: orange,
            // Dark base reads 8.38:1 on bright_yellow and 5.61:1 on orange.
            match_fg: bg,
        },
        accent: Accent {
            primary: mauve,
            alt: deep_mauve,
            link: cyan,
        },
        syntax: Syntax {
            inline_code: green,
            code_fg: fg,
            code_border: brown,
        },
        heading: Heading {
            h1: green,
            h2: cyan,
            h3: mauve,
            other: fg,
        },
        status: Status {
            // Darker than `raised`: the source theme's `dark_background`.
            bg: Color::Rgb(22, 26, 31), // #161a1f
            fg: gray,
            help_bg: bg,
            gutter: dark_fg,
        },
        list: List {
            marker: bright_yellow,
            task_marker: green,
            block_quote_fg: gray,
            block_quote_border: dark_fg,
        },
        table: Table {
            header: cyan,
            border: brown,
        },
        git: Git {
            new: green,
            modified: yellow,
        },
    }
}
