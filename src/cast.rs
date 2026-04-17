//! Saturating integer casts for terminal-UI arithmetic.
//!
//! Terminal dimensions and document line counts are bounded well below
//! `u16::MAX` / `u32::MAX` in any realistic scenario, but clippy's
//! `cast_possible_truncation` lint correctly flags bare `as` casts as
//! potentially lossy.  These helpers make the intent explicit and
//! saturate rather than silently wrap on overflow.

/// Saturating cast from `usize` to `u32`.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn u32_sat(n: usize) -> u32 {
    // Safety: n is clamped to u32::MAX before casting, so truncation is intentional.
    n.min(u32::MAX as usize) as u32
}

/// Saturating cast from `usize` to `u16`.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn u16_sat(n: usize) -> u16 {
    // Safety: n is clamped to u16::MAX before casting, so truncation is intentional.
    n.min(u16::MAX as usize) as u16
}

/// Saturating cast from `u32` to `u16`.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn u16_from_u32(n: u32) -> u16 {
    // Safety: n is clamped to u16::MAX before casting, so truncation is intentional.
    // Using `as u16` after `.min(u16::MAX as u32)` is clearer than `u16::try_from(...).unwrap()`.
    n.min(u32::from(u16::MAX)) as u16
}
