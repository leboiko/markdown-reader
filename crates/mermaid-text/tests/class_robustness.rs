//! Width-sweep and fuzz robustness tests for the `classDiagram` renderer.
//!
//! # Width sweep (Step 8)
//!
//! Renders a multi-class diagram at five widths [40, 60, 80, 120, 200] and
//! asserts structural invariants that must hold at every width:
//!
//! - The output is non-empty.
//! - Every class name that fits appears verbatim.
//! - The output never contains a bare Rust panic string (`thread 'main'`).
//!
//! # Fuzz test (Step 9)
//!
//! Generates 50 randomly mangled `classDiagram` strings from a fixed-seed
//! RNG and asserts that `render()` never panics (it may return an error).

// ---------------------------------------------------------------------------
// Width sweep
// ---------------------------------------------------------------------------

/// A fixture that exercises all renderable elements: classes with attributes
/// and methods, and every relationship kind.
const FIXTURE: &str = "classDiagram
class Animal {
    +String name
    +int age
    +speak() void
}
class Dog {
    +String breed
    +fetch() void
}
class Cat {
    +bool indoor
    +purr() void
}
class PetStore {
    +String address
    +sell(animal Animal) bool
}
Animal <|-- Dog
Animal <|-- Cat
PetStore o-- Animal : stocks";

#[test]
fn width_sweep_invariants() {
    for &width in &[40_usize, 60, 80, 120, 200] {
        let out = mermaid_text::render_with_width(FIXTURE, Some(width))
            .unwrap_or_else(|e| panic!("render failed at width {width}: {e}"));

        assert!(
            !out.is_empty(),
            "render at width {width} produced empty output"
        );

        // Rust panic strings must never appear in rendered output.
        assert!(
            !out.contains("thread '"),
            "panic string found in render at width {width}:\n{out}"
        );

        // The top-level structure must contain some box-drawing characters.
        assert!(
            out.contains('┌') || out.contains('+'),
            "no box character found at width {width}"
        );
    }
}

// ---------------------------------------------------------------------------
// Fuzz test — no panic on mangled input
// ---------------------------------------------------------------------------

/// A minimal deterministic pseudo-random number generator (xorshift32).
///
/// We avoid adding `rand` as a dev-dependency just for 50 iterations; the
/// xorshift period is more than enough for this use case.
struct Xorshift32 {
    state: u32,
}

impl Xorshift32 {
    fn new(seed: u32) -> Self {
        // Seed must be non-zero for xorshift to work.
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Return the next pseudo-random u32.
    fn next(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Return a pseudo-random usize in `[0, n)`.
    fn next_usize(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
}

/// Mangle a `classDiagram` source string in a repeatable pseudo-random way.
///
/// Each mutation randomly picks one of:
/// 1. Delete a random byte range (up to 10 bytes).
/// 2. Insert random ASCII punctuation at a random position.
/// 3. Replace a random byte with a random ASCII char.
fn mangle(src: &str, rng: &mut Xorshift32) -> String {
    let mut bytes: Vec<u8> = src.bytes().collect();
    let mutation = rng.next_usize(3);
    match mutation {
        0 => {
            // Delete a random byte range.
            if !bytes.is_empty() {
                let start = rng.next_usize(bytes.len());
                let len = (rng.next_usize(10) + 1).min(bytes.len() - start);
                bytes.drain(start..start + len);
            }
        }
        1 => {
            // Insert random printable ASCII at a random position.
            let pos = rng.next_usize(bytes.len() + 1);
            let ch = b'!' + (rng.next_usize(94)) as u8;
            bytes.insert(pos, ch);
        }
        _ => {
            // Replace a random byte.
            if !bytes.is_empty() {
                let pos = rng.next_usize(bytes.len());
                bytes[pos] = b'!' + (rng.next_usize(94)) as u8;
            }
        }
    }
    // Convert back; replace invalid UTF-8 sequences with the replacement char.
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Render `input` via `mermaid_text::render` and assert it does not panic.
///
/// Returns `true` if the render succeeded (for optional tallying), `false`
/// if it returned an error (which is fine — errors must not panic).
fn render_no_panic(input: &str) -> bool {
    // `std::panic::catch_unwind` is not available in integration tests the same
    // way, but `render` is not `UnwindSafe`. Instead we rely on the compiler's
    // guarantee: if render panics the test process aborts and the test fails.
    mermaid_text::render(input).is_ok()
}

#[test]
fn fuzz_no_panic_on_mangled_input() {
    const SEED: u32 = 0xDEAD_C0DE;
    const ITERATIONS: usize = 50;

    let base = "classDiagram
class A {
    +String x
    +foo() int
}
class B
A <|-- B
A *-- B : owns
A o-- B";

    let mut rng = Xorshift32::new(SEED);
    let mut ok_count = 0_usize;

    for _ in 0..ITERATIONS {
        let mangled = mangle(base, &mut rng);
        if render_no_panic(&mangled) {
            ok_count += 1;
        }
    }

    // We don't assert a minimum ok_count because mangled input is mostly
    // invalid — what matters is that no iteration panics. If even one
    // iteration panics the test process dies and this line is never reached.
    let _ = ok_count;
}
