use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// Selectable color themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Default,
    Dracula,
    SolarizedDark,
    SolarizedLight,
    Nord,
    GruvboxDark,
    GruvboxLight,
    GithubLight,
}

impl Theme {
    /// All selectable themes in display order.
    pub const ALL: &'static [Theme] = &[
        Theme::Default,
        Theme::Dracula,
        Theme::SolarizedDark,
        Theme::SolarizedLight,
        Theme::Nord,
        Theme::GruvboxDark,
        Theme::GruvboxLight,
        Theme::GithubLight,
    ];

    /// Human-readable display name for the theme.
    pub fn label(self) -> &'static str {
        match self {
            Theme::Default => "Default",
            Theme::Dracula => "Dracula",
            Theme::SolarizedDark => "Solarized Dark",
            Theme::SolarizedLight => "Solarized Light",
            Theme::Nord => "Nord",
            Theme::GruvboxDark => "Gruvbox Dark",
            Theme::GruvboxLight => "Gruvbox Light",
            Theme::GithubLight => "GitHub Light",
        }
    }

    /// Name of the bundled syntect theme to use for syntax highlighting when
    /// this UI theme is active.
    ///
    /// The returned string is always a key present in
    /// [`syntect::highlighting::ThemeSet::load_defaults`]'s output:
    /// - `"base16-ocean.dark"`
    /// - `"base16-eighties.dark"`
    /// - `"InspiredGitHub"`
    ///
    /// The exhaustive match (no `_` wildcard) ensures the compiler forces an
    /// update here whenever a new [`Theme`] variant is added.
    pub fn syntax_theme_name(self) -> &'static str {
        match self {
            Theme::Default | Theme::SolarizedDark | Theme::Nord => "base16-ocean.dark",
            Theme::Dracula | Theme::GruvboxDark => "base16-eighties.dark",
            Theme::SolarizedLight | Theme::GruvboxLight | Theme::GithubLight => "InspiredGitHub",
        }
    }
}

/// Concrete color values for one theme, threaded through every renderer.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub dim: Color,
    pub border: Color,
    pub border_focused: Color,
    pub accent: Color,
    pub accent_alt: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// Foreground color for text rendered on an `accent`-colored background.
    ///
    /// For most themes `selection_fg` happens to contrast adequately with
    /// `accent`, so they share the same value.  GitHub Light is the exception:
    /// its `selection_fg` is `#0969da` (same as `accent`), which would produce
    /// invisible blue-on-blue text.  Setting this field to white for that theme
    /// ensures readable text on the vivid blue accent background.
    pub on_accent_fg: Color,
    pub title: Color,
    pub h1: Color,
    pub h2: Color,
    pub h3: Color,
    pub heading_other: Color,
    pub inline_code: Color,
    pub code_fg: Color,
    pub code_bg: Color,
    pub code_border: Color,
    pub link: Color,
    pub list_marker: Color,
    pub task_marker: Color,
    pub block_quote_fg: Color,
    pub block_quote_border: Color,
    pub table_header: Color,
    pub table_border: Color,
    pub search_match_bg: Color,
    pub current_match_bg: Color,
    pub match_fg: Color,
    pub gutter: Color,
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub help_bg: Color,
    pub git_new: Color,
    pub git_modified: Color,
}

impl Palette {
    /// Construct the color palette for the given theme.
    #[allow(clippy::too_many_lines)]
    pub fn from_theme(theme: Theme) -> Self {
        match theme {
            Theme::Default => Self {
                // Neutral dark background, matches the existing hardcoded palette.
                background: Color::Rgb(20, 20, 30),
                foreground: Color::Rgb(220, 220, 220),
                dim: Color::DarkGray,
                border: Color::DarkGray,
                border_focused: Color::Cyan,
                accent: Color::Cyan,
                accent_alt: Color::Yellow,
                selection_bg: Color::Rgb(0, 160, 80),
                selection_fg: Color::Black,
                on_accent_fg: Color::Black,
                title: Color::Rgb(220, 220, 220),
                h1: Color::Cyan,
                h2: Color::Blue,
                h3: Color::Magenta,
                heading_other: Color::White,
                inline_code: Color::Green,
                code_fg: Color::Rgb(180, 200, 180),
                code_bg: Color::Rgb(40, 40, 40),
                code_border: Color::DarkGray,
                link: Color::Blue,
                list_marker: Color::Yellow,
                task_marker: Color::Cyan,
                block_quote_fg: Color::Gray,
                block_quote_border: Color::DarkGray,
                table_header: Color::Cyan,
                table_border: Color::DarkGray,
                search_match_bg: Color::Yellow,
                current_match_bg: Color::Rgb(255, 120, 0),
                match_fg: Color::Black,
                gutter: Color::DarkGray,
                status_bar_bg: Color::Rgb(30, 30, 30),
                status_bar_fg: Color::Gray,
                help_bg: Color::Rgb(20, 20, 30),
                git_new: Color::Rgb(80, 200, 120),
                git_modified: Color::Rgb(220, 180, 60),
            },
            Theme::Dracula => Self {
                // Official Dracula palette: https://draculatheme.com/contribute
                background: Color::Rgb(40, 42, 54), // background
                foreground: Color::Rgb(248, 248, 242), // foreground
                dim: Color::Rgb(98, 114, 164),      // comment
                border: Color::Rgb(68, 71, 90),     // current line
                border_focused: Color::Rgb(189, 147, 249), // purple
                accent: Color::Rgb(189, 147, 249),  // purple
                accent_alt: Color::Rgb(241, 250, 140), // yellow
                selection_bg: Color::Rgb(68, 71, 90), // current line
                selection_fg: Color::Rgb(248, 248, 242),
                // Dark text on bright purple so the accent button isn't blue-on-blue.
                // The original `foreground`-on-`purple` measured 2.26:1 (sub-AA).
                on_accent_fg: Color::Rgb(40, 42, 54), // background
                title: Color::Rgb(248, 248, 242),
                h1: Color::Rgb(255, 121, 198), // pink
                h2: Color::Rgb(189, 147, 249), // purple
                h3: Color::Rgb(80, 250, 123),  // green
                heading_other: Color::Rgb(248, 248, 242),
                inline_code: Color::Rgb(80, 250, 123), // green
                code_fg: Color::Rgb(248, 248, 242),
                code_bg: Color::Rgb(40, 42, 54),
                code_border: Color::Rgb(98, 114, 164),
                link: Color::Rgb(139, 233, 253),        // cyan
                list_marker: Color::Rgb(241, 250, 140), // yellow
                task_marker: Color::Rgb(80, 250, 123),  // green
                block_quote_fg: Color::Rgb(98, 114, 164),
                block_quote_border: Color::Rgb(98, 114, 164),
                table_header: Color::Rgb(255, 121, 198), // pink
                table_border: Color::Rgb(98, 114, 164),
                search_match_bg: Color::Rgb(241, 250, 140), // yellow
                current_match_bg: Color::Rgb(255, 121, 198), // pink
                match_fg: Color::Rgb(40, 42, 54),
                gutter: Color::Rgb(98, 114, 164),
                status_bar_bg: Color::Rgb(40, 42, 54),
                // Bumped from `comment` (98,114,164) which gave 3.03:1 vs status bg
                // to `foreground` so the status text actually reads at AA.
                status_bar_fg: Color::Rgb(248, 248, 242),
                help_bg: Color::Rgb(40, 42, 54),
                git_new: Color::Rgb(80, 250, 123),
                git_modified: Color::Rgb(241, 250, 140),
            },
            Theme::SolarizedDark => Self {
                // Ethan Schoonover's Solarized Dark: https://ethanschoonover.com/solarized/
                background: Color::Rgb(0, 43, 54),        // base03
                foreground: Color::Rgb(131, 148, 150),    // base0
                dim: Color::Rgb(88, 110, 117),            // base01
                border: Color::Rgb(88, 110, 117),         // base01
                border_focused: Color::Rgb(38, 139, 210), // blue
                accent: Color::Rgb(38, 139, 210),         // blue
                accent_alt: Color::Rgb(181, 137, 0),      // yellow
                // Was base02; identical to code_bg → cursor highlight invisible
                // inside code blocks. Lifted to base01 so the highlight reads
                // as a distinct surface tier; selection_fg promoted to base3
                // to keep AA on the new background.
                selection_bg: Color::Rgb(88, 110, 117), // base01
                selection_fg: Color::Rgb(253, 246, 227), // base3
                // base1 (147,161,161) on blue measured 1.38:1; pure black on
                // the vivid blue gives a clean ~6:1.
                on_accent_fg: Color::Rgb(0, 0, 0),
                title: Color::Rgb(147, 161, 161), // base1
                h1: Color::Rgb(203, 75, 22),      // orange
                h2: Color::Rgb(38, 139, 210),     // blue
                h3: Color::Rgb(42, 161, 152),     // cyan
                heading_other: Color::Rgb(131, 148, 150),
                inline_code: Color::Rgb(133, 153, 0), // green
                // base0 on base02 measured 4.11:1; bumped one notch to
                // base1 (147,161,161) for AA pass on code blocks.
                code_fg: Color::Rgb(147, 161, 161), // base1
                code_bg: Color::Rgb(7, 54, 66), // base02
                code_border: Color::Rgb(88, 110, 117),
                link: Color::Rgb(38, 139, 210),        // blue
                list_marker: Color::Rgb(181, 137, 0),  // yellow
                task_marker: Color::Rgb(42, 161, 152), // cyan
                block_quote_fg: Color::Rgb(88, 110, 117),
                block_quote_border: Color::Rgb(88, 110, 117),
                table_header: Color::Rgb(203, 75, 22), // orange
                table_border: Color::Rgb(88, 110, 117),
                search_match_bg: Color::Rgb(181, 137, 0), // yellow
                current_match_bg: Color::Rgb(203, 75, 22), // orange
                // True black so highlight text stays AA on both yellow
                // (search) and orange (current); base03 on orange was 3.26:1.
                match_fg: Color::Rgb(0, 0, 0),
                gutter: Color::Rgb(88, 110, 117),
                status_bar_bg: Color::Rgb(7, 54, 66),
                // Was base01 (88,110,117) → 2.42:1; bumped to base1.
                status_bar_fg: Color::Rgb(147, 161, 161), // base1
                help_bg: Color::Rgb(7, 54, 66),
                git_new: Color::Rgb(133, 153, 0),
                git_modified: Color::Rgb(181, 137, 0),
            },
            Theme::SolarizedLight => Self {
                // Ethan Schoonover's Solarized Light: https://ethanschoonover.com/solarized/
                //
                // Note: Solarized intentionally ships sub-AA contrast for a
                // "soft" reading look (base00 on base3 ≈ 4.13:1). For a
                // markdown reader where users actually parse text, we bump
                // primary text to base02 (much darker) so reading and code
                // blocks both reach AA. Accent slots stay canonical.
                background: Color::Rgb(253, 246, 227), // base3
                foreground: Color::Rgb(7, 54, 66),     // base02 (was base00, sub-AA)
                dim: Color::Rgb(101, 123, 131),        // base00 (was base1)
                border: Color::Rgb(238, 232, 213),     // base2
                border_focused: Color::Rgb(38, 139, 210), // blue
                accent: Color::Rgb(38, 139, 210),      // blue
                accent_alt: Color::Rgb(181, 137, 0),   // yellow
                // Was base2 → identical to code_bg → cursor highlight
                // invisible inside code blocks. Lifted to base1 (a darker
                // beige) so the highlight reads as a distinct tier.
                selection_bg: Color::Rgb(147, 161, 161), // base1
                selection_fg: Color::Rgb(7, 54, 66),     // base02
                // base3 on the vivid blue measured 3.41:1; pure white at
                // ~3.62:1 still fails. Use true black (~5:1).
                on_accent_fg: Color::Rgb(0, 0, 0),
                title: Color::Rgb(7, 54, 66),            // base02
                h1: Color::Rgb(203, 75, 22),             // orange
                h2: Color::Rgb(38, 139, 210),            // blue
                h3: Color::Rgb(42, 161, 152),            // cyan
                heading_other: Color::Rgb(7, 54, 66),    // base02
                inline_code: Color::Rgb(133, 153, 0),    // green
                code_fg: Color::Rgb(7, 54, 66),          // base02 (was base00, 3.64:1)
                code_bg: Color::Rgb(238, 232, 213),      // base2
                code_border: Color::Rgb(147, 161, 161),  // base1
                link: Color::Rgb(38, 139, 210),          // blue
                list_marker: Color::Rgb(181, 137, 0),    // yellow
                task_marker: Color::Rgb(42, 161, 152),   // cyan
                block_quote_fg: Color::Rgb(101, 123, 131), // base00
                block_quote_border: Color::Rgb(147, 161, 161),
                table_header: Color::Rgb(203, 75, 22), // orange
                table_border: Color::Rgb(147, 161, 161), // base1
                search_match_bg: Color::Rgb(181, 137, 0), // yellow
                current_match_bg: Color::Rgb(203, 75, 22), // orange
                // base3 on yellow/orange both sub-AA; true black passes both.
                match_fg: Color::Rgb(0, 0, 0),
                gutter: Color::Rgb(147, 161, 161),     // base1
                status_bar_bg: Color::Rgb(238, 232, 213), // base2
                status_bar_fg: Color::Rgb(7, 54, 66),     // base02 (was base00, 3.64:1)
                help_bg: Color::Rgb(238, 232, 213),    // base2
                git_new: Color::Rgb(133, 153, 0),      // green
                git_modified: Color::Rgb(181, 137, 0), // yellow
            },
            Theme::Nord => Self {
                // Arctic, north-bluish color palette: https://www.nordtheme.com/docs/colors-and-palettes
                background: Color::Rgb(46, 52, 64),        // nord0
                foreground: Color::Rgb(216, 222, 233),     // nord4
                dim: Color::Rgb(76, 86, 106),              // nord2
                border: Color::Rgb(67, 76, 94),            // nord1
                border_focused: Color::Rgb(136, 192, 208), // nord8
                accent: Color::Rgb(136, 192, 208),         // nord8
                accent_alt: Color::Rgb(235, 203, 139),     // nord13 yellow
                selection_bg: Color::Rgb(67, 76, 94),      // nord1
                selection_fg: Color::Rgb(236, 239, 244),   // nord6
                on_accent_fg: Color::Rgb(46, 52, 64),
                title: Color::Rgb(236, 239, 244), // nord6
                h1: Color::Rgb(191, 97, 106),     // nord11 red
                h2: Color::Rgb(136, 192, 208),    // nord8
                h3: Color::Rgb(163, 190, 140),    // nord14 green
                heading_other: Color::Rgb(216, 222, 233),
                inline_code: Color::Rgb(163, 190, 140), // nord14 green
                code_fg: Color::Rgb(216, 222, 233),
                code_bg: Color::Rgb(59, 66, 82), // nord3
                code_border: Color::Rgb(76, 86, 106),
                link: Color::Rgb(129, 161, 193),        // nord9
                list_marker: Color::Rgb(235, 203, 139), // nord13
                task_marker: Color::Rgb(163, 190, 140), // nord14
                block_quote_fg: Color::Rgb(76, 86, 106),
                block_quote_border: Color::Rgb(76, 86, 106),
                table_header: Color::Rgb(94, 129, 172), // nord10 blue
                table_border: Color::Rgb(76, 86, 106),
                search_match_bg: Color::Rgb(235, 203, 139), // nord13
                current_match_bg: Color::Rgb(191, 97, 106), // nord11
                // nord0 on nord11 (red) measured 3.05:1; true black gets ~5:1.
                match_fg: Color::Rgb(0, 0, 0),
                gutter: Color::Rgb(76, 86, 106),
                status_bar_bg: Color::Rgb(59, 66, 82),
                // Was nord2 (76,86,106) on nord3 (59,66,82) → 1.36:1, basically
                // illegible. Bumped to nord4 (the foreground) for clean AA.
                status_bar_fg: Color::Rgb(216, 222, 233), // nord4
                help_bg: Color::Rgb(59, 66, 82),
                git_new: Color::Rgb(163, 190, 140),
                git_modified: Color::Rgb(235, 203, 139),
            },
            Theme::GruvboxDark => Self {
                // Gruvbox Dark: https://github.com/morhetz/gruvbox
                background: Color::Rgb(40, 40, 40),      // bg
                foreground: Color::Rgb(235, 219, 178),   // fg
                dim: Color::Rgb(146, 131, 116),          // gray
                border: Color::Rgb(80, 73, 69),          // bg3
                border_focused: Color::Rgb(214, 93, 14), // orange (bright)
                accent: Color::Rgb(250, 189, 47),        // yellow (bright)
                accent_alt: Color::Rgb(184, 187, 38),    // green
                selection_bg: Color::Rgb(80, 73, 69),    // bg3
                selection_fg: Color::Rgb(235, 219, 178),
                on_accent_fg: Color::Rgb(40, 40, 40),
                title: Color::Rgb(235, 219, 178),
                h1: Color::Rgb(251, 73, 52),  // red (bright)
                h2: Color::Rgb(250, 189, 47), // yellow (bright)
                h3: Color::Rgb(184, 187, 38), // green (bright)
                heading_other: Color::Rgb(235, 219, 178),
                inline_code: Color::Rgb(184, 187, 38), // green
                code_fg: Color::Rgb(235, 219, 178),
                code_bg: Color::Rgb(50, 48, 47), // bg1
                code_border: Color::Rgb(80, 73, 69),
                link: Color::Rgb(131, 165, 152),       // aqua
                list_marker: Color::Rgb(250, 189, 47), // yellow
                task_marker: Color::Rgb(184, 187, 38), // green
                block_quote_fg: Color::Rgb(146, 131, 116),
                block_quote_border: Color::Rgb(146, 131, 116),
                table_header: Color::Rgb(214, 93, 14), // orange
                table_border: Color::Rgb(80, 73, 69),
                search_match_bg: Color::Rgb(250, 189, 47), // yellow
                current_match_bg: Color::Rgb(251, 73, 52), // red
                // bg (40,40,40) on red (251,73,52) measured 4.29:1, just under
                // AA; true black takes it cleanly over.
                match_fg: Color::Rgb(0, 0, 0),
                gutter: Color::Rgb(102, 92, 84), // bg4
                status_bar_bg: Color::Rgb(50, 48, 47),
                // Was gray (146,131,116) → 3.58:1; bumped to fg for AA.
                status_bar_fg: Color::Rgb(235, 219, 178), // fg
                help_bg: Color::Rgb(50, 48, 47),
                git_new: Color::Rgb(184, 187, 38),
                git_modified: Color::Rgb(250, 189, 47),
            },
            Theme::GruvboxLight => Self {
                // Gruvbox Light: https://github.com/morhetz/gruvbox
                background: Color::Rgb(251, 241, 199), // bg  #fbf1c7
                foreground: Color::Rgb(60, 56, 54),    // fg  #3c3836
                dim: Color::Rgb(146, 131, 116),        // gray #928374
                border: Color::Rgb(213, 196, 161),     // bg2 #d5c4a1
                border_focused: Color::Rgb(214, 93, 14), // orange
                accent: Color::Rgb(215, 153, 33),      // yellow #d79921
                accent_alt: Color::Rgb(152, 151, 26),  // green #98971a
                // Was bg1 → identical to code_bg → cursor highlight invisible
                // inside code blocks. Lifted to bg2 (a darker beige) so the
                // highlight reads as a distinct tier.
                selection_bg: Color::Rgb(213, 196, 161), // bg2
                selection_fg: Color::Rgb(60, 56, 54),  // fg
                on_accent_fg: Color::Rgb(60, 56, 54),
                title: Color::Rgb(60, 56, 54),
                h1: Color::Rgb(204, 36, 29),  // red #cc241d
                h2: Color::Rgb(215, 153, 33), // yellow
                h3: Color::Rgb(152, 151, 26), // green
                heading_other: Color::Rgb(60, 56, 54),
                inline_code: Color::Rgb(177, 98, 134), // purple #b16286
                code_fg: Color::Rgb(60, 56, 54),
                code_bg: Color::Rgb(235, 219, 178),     // bg1
                code_border: Color::Rgb(213, 196, 161), // bg2
                link: Color::Rgb(69, 133, 136),         // blue #458588
                list_marker: Color::Rgb(215, 153, 33),  // yellow
                task_marker: Color::Rgb(104, 157, 106), // aqua #689d6a
                block_quote_fg: Color::Rgb(146, 131, 116),
                block_quote_border: Color::Rgb(213, 196, 161),
                table_header: Color::Rgb(214, 93, 14), // orange
                table_border: Color::Rgb(213, 196, 161), // bg2
                search_match_bg: Color::Rgb(215, 153, 33), // yellow
                current_match_bg: Color::Rgb(214, 93, 14), // orange
                // Light bg on yellow/orange both sub-AA; true black on yellow
                // measured ~10:1, on orange ~5:1.
                match_fg: Color::Rgb(0, 0, 0),
                gutter: Color::Rgb(146, 131, 116),     // gray
                status_bar_bg: Color::Rgb(235, 219, 178), // bg1
                status_bar_fg: Color::Rgb(80, 73, 69), // fg1 #504945
                help_bg: Color::Rgb(235, 219, 178),    // bg1
                git_new: Color::Rgb(152, 151, 26),     // green
                git_modified: Color::Rgb(215, 153, 33), // yellow
            },
            Theme::GithubLight => Self {
                // GitHub Light: https://primer.style/primitives/colors
                background: Color::Rgb(255, 255, 255), // canvas.default   #ffffff
                foreground: Color::Rgb(31, 35, 40),    // fg.default        #1f2328
                dim: Color::Rgb(101, 109, 118),        // fg.muted          #656d76
                border: Color::Rgb(208, 215, 222),     // border.default    #d0d7de
                border_focused: Color::Rgb(9, 105, 218), // accent.fg         #0969da
                accent: Color::Rgb(9, 105, 218),       // accent.fg         #0969da
                accent_alt: Color::Rgb(154, 103, 0),   // attention.fg      #9a6700
                selection_bg: Color::Rgb(221, 244, 255), // accent.subtle     #ddf4ff
                selection_fg: Color::Rgb(9, 105, 218), // accent.fg on subtle bg
                // White on the vivid blue — selection_fg is also #0969da which would
                // produce invisible blue-on-blue text if used on an accent background.
                on_accent_fg: Color::Rgb(255, 255, 255),
                title: Color::Rgb(31, 35, 40),
                h1: Color::Rgb(9, 105, 218), // accent.fg         #0969da
                h2: Color::Rgb(154, 103, 0), // attention.fg      #9a6700
                h3: Color::Rgb(26, 127, 55), // success.fg        #1a7f37
                heading_other: Color::Rgb(31, 35, 40),
                inline_code: Color::Rgb(207, 34, 46), // danger.fg         #cf222e
                code_fg: Color::Rgb(31, 35, 40),
                code_bg: Color::Rgb(246, 248, 250), // canvas.subtle     #f6f8fa
                code_border: Color::Rgb(208, 215, 222), // border.default    #d0d7de
                link: Color::Rgb(9, 105, 218),      // accent.fg         #0969da
                list_marker: Color::Rgb(154, 103, 0), // attention.fg      #9a6700
                task_marker: Color::Rgb(26, 127, 55), // success.fg        #1a7f37
                block_quote_fg: Color::Rgb(101, 109, 118),
                block_quote_border: Color::Rgb(208, 215, 222),
                table_header: Color::Rgb(9, 105, 218),
                table_border: Color::Rgb(208, 215, 222),
                search_match_bg: Color::Rgb(255, 211, 61), // attention.emphasis #ffd33d
                current_match_bg: Color::Rgb(255, 143, 0), // severe.emphasis   #ff8f00
                match_fg: Color::Rgb(31, 35, 40),
                gutter: Color::Rgb(101, 109, 118),
                status_bar_bg: Color::Rgb(246, 248, 250), // canvas.subtle     #f6f8fa
                status_bar_fg: Color::Rgb(101, 109, 118),
                help_bg: Color::Rgb(246, 248, 250), // canvas.subtle     #f6f8fa
                git_new: Color::Rgb(26, 127, 55),   // success.fg        #1a7f37
                git_modified: Color::Rgb(154, 103, 0), // attention.fg      #9a6700
            },
        }
    }

    /// Style for unfocused panel borders.
    pub fn border_style(self) -> Style {
        Style::new().fg(self.border)
    }

    /// Style for focused panel borders.
    pub fn border_focused_style(self) -> Style {
        Style::new().fg(self.border_focused)
    }

    /// Bold style for widget titles.
    pub fn title_style(self) -> Style {
        Style::new().fg(self.title).add_modifier(Modifier::BOLD)
    }

    /// Style for the currently selected list item.
    pub fn selected_style(self) -> Style {
        Style::new()
            .bg(self.selection_bg)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for de-emphasized (dim) text.
    pub fn dim_style(self) -> Style {
        Style::new().fg(self.dim)
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::from_theme(Theme::Default)
    }
}

// ── Contrast / palette-invariant audit ───────────────────────────────────────
//
// The audit is intentionally *not* exhaustive over every Palette field —
// it targets pairs the user actually reads (text on its background, search
// highlight, status bar, etc.). Pure decoration (border colours, gutter)
// is excluded from the AA threshold because thin lines tolerate lower
// contrast and pinning them inflates theme rejections without visible win.
//
// Two invariant categories:
//   * **Equality** — same Color in two slots that should be visually
//     distinct (e.g. selection_bg == code_bg → cursor highlight invisible
//     inside code blocks). The 0.16-era solarized_light selection bug.
//   * **Contrast** — WCAG AA contrast ratio (≥ 4.5:1 for normal text).
//     Only computed for RGB-defined slots; named colours (`Color::Cyan`
//     etc.) skip silently because their on-screen RGB is terminal-defined.
//
// Add new pairs to `READING_PAIRS` (contrast) or extend the equality test
// when a new Palette field appears whose collision would hide content.

/// Convert a ratatui [`Color`] to approximate sRGB (0-255). Returns
/// `None` for indexed/reset/named colours whose on-screen RGB is
/// terminal-defined and so can't be evaluated objectively.
fn color_to_srgb(c: Color) -> Option<(u8, u8, u8)> {
    match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

/// WCAG relative luminance of an sRGB triple (per WCAG 2.1).
fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    fn channel(c: u8) -> f64 {
        let s = f64::from(c) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// WCAG contrast ratio between two sRGB colours (per WCAG 2.1 SC 1.4.3).
/// Returns `None` if either input is a non-RGB ratatui colour.
#[cfg_attr(not(test), allow(dead_code))]
fn contrast_ratio(fg: Color, bg: Color) -> Option<f64> {
    let l1 = relative_luminance(color_to_srgb(fg)?);
    let l2 = relative_luminance(color_to_srgb(bg)?);
    let (light, dark) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    Some((light + 0.05) / (dark + 0.05))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG AA threshold for normal-size text. Smaller text and thin
    /// strokes (e.g. box-drawing borders) want this floor too — sub-AA
    /// is the symptom that triggered the 1.22.4 fix conversation.
    const AA_NORMAL: f64 = 4.5;

    /// Every theme must have `on_accent_fg != accent` so that text drawn on an
    /// accent-coloured background is never invisible.
    #[test]
    fn on_accent_fg_contrasts_with_accent() {
        for &theme in Theme::ALL {
            let p = Palette::from_theme(theme);
            assert_ne!(
                p.on_accent_fg, p.accent,
                "Theme {theme:?}: on_accent_fg == accent — text would be invisible",
            );
        }
    }

    /// Selection / cursor highlight backgrounds must differ from the
    /// surfaces they overlay, otherwise the highlight is invisible.
    ///
    /// Reported on solarized_light 2026-04-24: `selection_bg ==
    /// code_bg == Rgb(238, 232, 213)` made the cursor highlight inside
    /// code blocks completely invisible.
    #[test]
    fn highlight_bgs_differ_from_surfaces() {
        let mut failures: Vec<String> = Vec::new();
        for &theme in Theme::ALL {
            let p = Palette::from_theme(theme);
            for (a_name, a, b_name, b) in [
                ("selection_bg", p.selection_bg, "code_bg", p.code_bg),
                ("selection_bg", p.selection_bg, "background", p.background),
                ("current_match_bg", p.current_match_bg, "code_bg", p.code_bg),
                ("current_match_bg", p.current_match_bg, "background", p.background),
            ] {
                if a == b {
                    failures.push(format!(
                        "  {theme:?}: {a_name} == {b_name} ({a:?}) — highlight invisible",
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "highlight backgrounds collide with surfaces:\n{}",
            failures.join("\n"),
        );
    }

    /// Reading-text fg/bg pairs must meet WCAG AA contrast for normal
    /// text (≥ 4.5:1). Named colours (terminal-defined RGB) skip
    /// silently — only RGB-defined pairs are asserted.
    #[test]
    fn reading_text_meets_wcag_aa() {
        let mut failures: Vec<String> = Vec::new();
        for &theme in Theme::ALL {
            let p = Palette::from_theme(theme);
            // (label, fg, bg) — pairs the user actually reads as text.
            // Decoration (borders, gutters) is intentionally excluded.
            let pairs: &[(&str, Color, Color)] = &[
                ("foreground/background", p.foreground, p.background),
                ("code_fg/code_bg", p.code_fg, p.code_bg),
                ("selection_fg/selection_bg", p.selection_fg, p.selection_bg),
                ("on_accent_fg/accent", p.on_accent_fg, p.accent),
                ("match_fg/search_match_bg", p.match_fg, p.search_match_bg),
                ("match_fg/current_match_bg", p.match_fg, p.current_match_bg),
                ("status_bar_fg/status_bar_bg", p.status_bar_fg, p.status_bar_bg),
            ];
            for (name, fg, bg) in pairs {
                if let Some(ratio) = contrast_ratio(*fg, *bg)
                    && ratio < AA_NORMAL
                {
                    failures.push(format!(
                        "  {theme:?} {name}: {ratio:.2}:1 < {AA_NORMAL}:1",
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "reading-text pairs fail WCAG AA:\n{}",
            failures.join("\n"),
        );
    }

    /// Sanity-check the contrast formula against published WCAG examples.
    #[test]
    fn contrast_ratio_matches_known_values() {
        // White on black is the maximum possible: 21:1.
        let r = contrast_ratio(Color::Rgb(255, 255, 255), Color::Rgb(0, 0, 0)).unwrap();
        assert!((r - 21.0).abs() < 0.01, "white/black: {r:.4}");
        // Same colour on itself is the minimum: 1:1.
        let r = contrast_ratio(Color::Rgb(128, 128, 128), Color::Rgb(128, 128, 128)).unwrap();
        assert!((r - 1.0).abs() < 0.01, "self/self: {r:.4}");
        // Non-RGB inputs return None (caller skips silently).
        assert_eq!(contrast_ratio(Color::Cyan, Color::Black), None);
    }
}
