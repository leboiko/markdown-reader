//! Best-effort LaTeX math → Unicode text conversion.
//!
//! Converts common LaTeX math commands to their Unicode equivalents so
//! display and inline math render readably in a terminal without a full
//! typesetting engine.  The output is plain text — no rasterisation, no
//! external dependencies.
//!
//! Coverage: Greek letters, common operators and relations, arrows,
//! `\frac`, `\sqrt`, super/subscripts (digits + a few letters), and
//! delimiter commands (`\left`, `\right`, `\cdots`, etc.).  Unknown
//! commands pass through stripped of their backslash so the result is
//! always at least as readable as raw LaTeX.

/// Convert a LaTeX math expression to approximate Unicode text.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(latex_to_unicode(r"E = mc^2"), "E = mc²");
/// assert_eq!(latex_to_unicode(r"\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ xᵢ");
/// ```
#[allow(clippy::too_many_lines)]
pub fn latex_to_unicode(input: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        match chars[i] {
            '\\' => {
                // Collect the command name (letters only).
                let start = i + 1;
                let mut end = start;
                while end < len && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }
                if end == start {
                    // Single non-alpha char after backslash: \{, \}, \\, etc.
                    if start < len {
                        match chars[start] {
                            // Bare braces or negative thin space → skip silently.
                            '{' | '}' | '!' => {
                                i = start + 1;
                                continue;
                            }
                            '\\' => {
                                out.push('\n');
                                i = start + 1;
                                continue;
                            }
                            ',' => {
                                // Thin space.
                                out.push('\u{2009}');
                                i = start + 1;
                                continue;
                            }
                            ';' => {
                                out.push(' ');
                                i = start + 1;
                                continue;
                            }
                            _ => {
                                out.push(chars[start]);
                                i = start + 1;
                                continue;
                            }
                        }
                    }
                    i += 1;
                    continue;
                }
                let cmd: String = chars[start..end].iter().collect();
                i = end;

                // Try to map the command.
                if let Some(sym) = command_to_unicode(&cmd) {
                    out.push_str(sym);
                } else if cmd == "frac" {
                    let num = extract_brace_group(&chars, &mut i);
                    let den = extract_brace_group(&chars, &mut i);
                    let num_u = latex_to_unicode(&num);
                    let den_u = latex_to_unicode(&den);
                    if num_u.len() <= 1 && den_u.len() <= 1 {
                        let _ = write!(out, "{num_u}/{den_u}");
                    } else {
                        let _ = write!(out, "({num_u})/({den_u})");
                    }
                } else if cmd == "sqrt" {
                    let body = extract_brace_group(&chars, &mut i);
                    let body_u = latex_to_unicode(&body);
                    let _ = write!(out, "√({body_u})");
                } else if cmd == "text"
                    || cmd == "mathrm"
                    || cmd == "mathbf"
                    || cmd == "mathit"
                    || cmd == "mathbb"
                    || cmd == "mathcal"
                    || cmd == "operatorname"
                {
                    let body = extract_brace_group(&chars, &mut i);
                    out.push_str(&latex_to_unicode(&body));
                } else if cmd == "left" || cmd == "right" {
                    // Consume the delimiter character that follows.
                    if i < len {
                        match chars[i] {
                            '(' | ')' | '[' | ']' | '|' => {
                                out.push(chars[i]);
                                i += 1;
                            }
                            '\\' if i + 1 < len => {
                                // \left\{ or \right\}
                                if chars[i + 1] == '{' {
                                    out.push('(');
                                } else if chars[i + 1] == '}' {
                                    out.push(')');
                                } else if chars[i + 1] == '|' {
                                    out.push('‖');
                                }
                                i += 2;
                            }
                            '.' => {
                                i += 1;
                            } // \left. or \right. = invisible delimiter
                            _ => {}
                        }
                    }
                } else if cmd == "begin" || cmd == "end" {
                    // Skip \begin{env} / \end{env}.
                    let _env = extract_brace_group(&chars, &mut i);
                } else {
                    // Unknown command — emit the name without the backslash.
                    out.push_str(&cmd);
                }
            }
            '{' | '}' => {
                // Bare braces used for grouping — skip.
                i += 1;
            }
            '^' => {
                i += 1;
                let group = extract_script_arg(&chars, &mut i);
                let converted = latex_to_unicode(&group);
                for ch in converted.chars() {
                    out.push(to_superscript(ch));
                }
            }
            '_' => {
                i += 1;
                let group = extract_script_arg(&chars, &mut i);
                let converted = latex_to_unicode(&group);
                for ch in converted.chars() {
                    out.push(to_subscript(ch));
                }
            }
            '~' => {
                out.push(' ');
                i += 1;
            }
            _ => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// Extract the argument of a `^` or `_` script operator.
///
/// Handles three forms:
/// - `{…}` brace group
/// - `\command` (a backslash followed by alphabetic chars)
/// - A single character
fn extract_script_arg(chars: &[char], pos: &mut usize) -> String {
    // Skip whitespace.
    while *pos < chars.len() && chars[*pos] == ' ' {
        *pos += 1;
    }
    if *pos >= chars.len() {
        return String::new();
    }
    if chars[*pos] == '{' {
        extract_brace_group(chars, pos)
    } else if chars[*pos] == '\\' {
        // Capture the entire \command as the script argument.
        let start = *pos;
        *pos += 1; // skip '\'
        while *pos < chars.len() && chars[*pos].is_ascii_alphabetic() {
            *pos += 1;
        }
        chars[start..*pos].iter().collect()
    } else {
        let c = chars[*pos];
        *pos += 1;
        c.to_string()
    }
}

/// Extract a `{…}` brace-delimited group starting at `chars[*pos]`.
/// If `chars[*pos]` is not `{`, returns an empty string.
/// Advances `*pos` past the closing `}`.
fn extract_brace_group(chars: &[char], pos: &mut usize) -> String {
    // Skip whitespace before the brace.
    while *pos < chars.len() && chars[*pos] == ' ' {
        *pos += 1;
    }
    if *pos >= chars.len() || chars[*pos] != '{' {
        return String::new();
    }
    *pos += 1; // skip '{'
    let mut depth = 1;
    let start = *pos;
    while *pos < chars.len() && depth > 0 {
        match chars[*pos] {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            *pos += 1;
        }
    }
    let end = *pos;
    if *pos < chars.len() {
        *pos += 1; // skip closing '}'
    }
    chars[start..end].iter().collect()
}

/// Map a LaTeX command name (without backslash) to a Unicode string.
#[allow(clippy::too_many_lines)]
fn command_to_unicode(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        // Greek lowercase
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        // Greek uppercase
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        // Operators
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "iiint" => "∭",
        "oint" => "∮",
        "coprod" => "∐",
        "bigcup" => "⋃",
        "bigcap" => "⋂",
        "bigoplus" | "oplus" => "⊕",
        "bigotimes" | "otimes" => "⊗",
        // Relations
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "propto" => "∝",
        "ll" => "≪",
        "gg" => "≫",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "emptyset" | "varnothing" => "∅",
        // Arrows
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" => "⇔",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "mapsto" => "↦",
        "hookrightarrow" => "↪",
        "longrightarrow" => "⟶",
        "longleftarrow" => "⟵",
        "Longrightarrow" | "implies" => "⟹",
        "iff" => "⟺",
        // Miscellaneous
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "pm" => "±",
        "mp" => "∓",
        "times" => "×",
        "div" => "÷",
        "cdot" => "·",
        "star" => "⋆",
        "ast" => "∗",
        "circ" => "∘",
        "bullet" => "•",
        "dagger" => "†",
        "ddagger" => "‡",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "cap" => "∩",
        "cup" => "∪",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "angle" => "∠",
        "measuredangle" => "∡",
        "perp" => "⊥",
        "parallel" => "∥",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "wp" => "℘",
        // Blackboard bold letters (common)
        "mathbb" => "", // handled separately via extract_brace_group
        // Spacing
        "quad" => "  ",
        "qquad" => "    ",
        // Misc text
        "langle" => "⟨",
        "rangle" => "⟩",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        _ => return None,
    })
}

/// Best-effort mapping of a character to its Unicode superscript form.
fn to_superscript(ch: char) -> char {
    match ch {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'z' => 'ᶻ',
        'T' => 'ᵀ',
        _ => ch, // no superscript form available — pass through
    }
}

/// Best-effort mapping of a character to its Unicode subscript form.
fn to_subscript(ch: char) -> char {
    match ch {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_superscript() {
        assert_eq!(latex_to_unicode("E = mc^2"), "E = mc²");
    }

    #[test]
    fn simple_subscript() {
        assert_eq!(latex_to_unicode("x_i"), "xᵢ");
    }

    #[test]
    fn greek_letters() {
        assert_eq!(latex_to_unicode(r"\alpha + \beta"), "α + β");
    }

    #[test]
    fn fraction() {
        assert_eq!(latex_to_unicode(r"\frac{a}{b}"), "a/b");
        assert_eq!(latex_to_unicode(r"\frac{x+1}{y-1}"), "(x+1)/(y-1)");
    }

    #[test]
    fn sqrt() {
        assert_eq!(latex_to_unicode(r"\sqrt{x}"), "√(x)");
    }

    #[test]
    fn sum_with_limits() {
        let result = latex_to_unicode(r"\sum_{i=1}^{n} x_i");
        assert!(result.contains('∑'), "should contain sum symbol: {result}");
        assert!(
            result.contains('ₙ') || result.contains('ⁿ'),
            "should have n: {result}"
        );
    }

    #[test]
    fn euler_identity() {
        let result = latex_to_unicode(r"e^{i\pi} + 1 = 0");
        assert!(result.contains('π'), "should contain pi: {result}");
        assert!(result.contains('ⁱ'), "should have superscript i: {result}");
    }

    #[test]
    fn integral() {
        let result = latex_to_unicode(r"\int_0^\infty e^{-x} dx");
        assert!(result.contains('∫'), "should contain integral: {result}");
        assert!(result.contains('∞'), "should contain infinity: {result}");
    }

    #[test]
    fn nabla_and_partial() {
        let result = latex_to_unicode(r"\nabla \cdot \mathbf{E} = \frac{\rho}{\varepsilon_0}");
        assert!(result.contains('∇'), "should contain nabla: {result}");
        assert!(result.contains('·'), "should contain cdot: {result}");
    }

    #[test]
    fn unknown_command_passes_through() {
        let result = latex_to_unicode(r"\unknowncmd{x}");
        assert!(
            result.contains("unknowncmd"),
            "unknown command should pass through: {result}"
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(latex_to_unicode(""), "");
    }
}
