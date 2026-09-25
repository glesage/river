//! What the loading dots look like, with no opinion on when they show.
//!
//! - **Wave dots**: ten accent-blue dots riding one travelling wave.
//! - **Small dots**: five 3 px dots bobbing in a wave, as in the connection
//!   pill.
//!
//! `network_activity_indicator` decides when its gated dots show and draws
//! them with the markup helpers here. [`WaveDots`] and [`SmallDots`] are
//! always on: for a surface that is loading for exactly as long as it is
//! mounted (the rooms rail and the no-room screen while rooms load, a room
//! waiting for its first sync, an invite DM on its way).

use dioxus::prelude::*;

/// Dots in a row of wave dots.
const WAVE_DOT_COUNT: usize = 10;

/// Dots in a row of small dots. main.css's open `.pill-activity` width must
/// fit exactly this many (see `pill_width_fits_the_dots`).
const SMALL_DOT_COUNT: usize = 5;

/// One row's worth of wave dots, for a container with class `river-flow`.
/// `seed` picks the wave: keep it stable for as long as the row is up, so a
/// re-render doesn't restart the animation.
pub(crate) fn wave_dot_spans(seed: u64, dot_testid: &'static str) -> Element {
    let motions = dot_motions(seed);
    rsx! {
        for (i, motion) in motions.iter().enumerate() {
            span {
                key: "{i}",
                class: "river-flow-dot",
                "data-testid": dot_testid,
                style: motion.style(i),
            }
        }
    }
}

/// The five small dots, for a container with class `pill-activity` or
/// `small-dots`.
pub(crate) fn small_dot_spans() -> Element {
    rsx! {
        for i in 0..SMALL_DOT_COUNT {
            span { key: "{i}", class: "pill-activity-dot", style: "--i: {i};" }
        }
    }
}

/// Big wave dots, in the flow, for a surface that is loading for as long as
/// it is mounted (the rooms rail and the no-room screen while rooms load).
#[component]
pub fn WaveDots(testid: &'static str) -> Element {
    // One wave per mount, kept across re-renders so the animation doesn't
    // restart.
    let seed = use_hook(|| crate::components::app::sync_info::now_ms().to_bits());
    rsx! {
        div {
            class: "river-flow",
            "aria-hidden": "true",
            "data-testid": testid,
            {wave_dot_spans(seed, "wave-dot")}
        }
    }
}

/// The connection pill's small dots, in accent blue, for an inline "this is
/// in progress" (a room row, a banner, a footer).
#[component]
pub fn SmallDots(testid: &'static str) -> Element {
    rsx! {
        span {
            class: "small-dots text-accent",
            "aria-hidden": "true",
            "data-testid": testid,
            {small_dot_spans()}
        }
    }
}

// ---- Wave motion ----------------------------------------------------------
//
// Every dot rides the same 1.5 s travelling wave (`river-flow-wave` in
// main.css), so the crest still reads as one ripple crossing the row. What
// varies per dot, so it moves like water rather than a metronome: how high it
// rises, a nudge to its place on the wave, which smooth curve it eases along,
// and a slower second swell on its own period. The periods do not divide each other, so the row never exactly
// repeats. Every curve is a monotone cubic-bezier: no steps, no linear, no
// overshoot.

/// Seconds between neighbouring dots on the travelling wave. Must match the
/// `0.15s` in main.css's `animation-delay` for `.river-flow-dot`.
const WAVE_STEP_S: f64 = 0.15;
/// Primary rise, px either side of rest.
const AMP_PX: (f64, f64) = (2.0, 3.0);
/// Largest nudge to a dot's place on the wave. Kept under half of
/// `WAVE_STEP_S` so the crest still travels strictly left to right.
const PHASE_JITTER_S: f64 = 0.045;
/// Secondary swell, px either side, and its period.
const SWELL_PX: (f64, f64) = (0.6, 1.4);
const SWELL_PERIOD_S: (f64, f64) = (2.2, 3.4);
/// Height of the `.river-flow` row in main.css. The motion must fit inside it,
/// because the 12 px clearances above and below are measured from its edges.
const ROW_HEIGHT_PX: f64 = 14.0;
/// Half a 5 px dot at the crest's `scale: 1`.
const DOT_RADIUS_PX: f64 = 2.5;

/// Smooth S-curves the dots ease along. Control points keep y at 0 and 1, so
/// each is monotone with no overshoot: every dot always moves on a curve.
const WAVE_EASES: [&str; 6] = [
    "cubic-bezier(0.37, 0, 0.63, 1)",
    "cubic-bezier(0.45, 0, 0.55, 1)",
    "cubic-bezier(0.65, 0, 0.35, 1)",
    "cubic-bezier(0.42, 0, 0.58, 1)",
    "cubic-bezier(0.3, 0, 0.5, 1)",
    "cubic-bezier(0.5, 0, 0.7, 1)",
];

/// One dot's share of the wave, emitted as custom properties that main.css's
/// keyframes read.
#[derive(Clone, Copy, PartialEq, Debug)]
struct DotMotion {
    amp_px: f64,
    phase_jitter_s: f64,
    ease: &'static str,
    swell_px: f64,
    swell_period_s: f64,
    /// Negative, so the swell is already under way on the first frame.
    swell_delay_s: f64,
}

impl DotMotion {
    fn style(&self, i: usize) -> String {
        format!(
            "--i: {i}; --amp: {:.2}px; --jitter: {:.3}s; --ease: {}; --swell: {:.2}px; \
             --swell-dur: {:.2}s; --swell-delay: {:.2}s;",
            self.amp_px,
            self.phase_jitter_s,
            self.ease,
            self.swell_px,
            self.swell_period_s,
            self.swell_delay_s,
        )
    }
}

/// The wave for one appearance of the dots. Deterministic in `seed`, so the
/// same appearance renders the same wave on every re-render.
fn dot_motions(seed: u64) -> [DotMotion; WAVE_DOT_COUNT] {
    let mut rng = SplitMix64(seed);
    std::array::from_fn(|_| {
        let swell_period_s = rng.range(SWELL_PERIOD_S);
        DotMotion {
            amp_px: rng.range(AMP_PX),
            phase_jitter_s: rng.range((-PHASE_JITTER_S, PHASE_JITTER_S)),
            ease: WAVE_EASES[(rng.next_u64() % WAVE_EASES.len() as u64) as usize],
            swell_px: rng.range(SWELL_PX),
            swell_period_s,
            swell_delay_s: -rng.range((0.0, swell_period_s)),
        }
    })
}

/// splitmix64: tiny, fast, and plenty for choosing how dots wobble.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[lo, hi)`.
    fn range(&mut self, (lo, hi): (f64, f64)) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loading is shown with dots (`WaveDots`, `SmallDots`, or the gated
    /// indicators), never a spinner: one visual language for "in progress".
    #[test]
    fn no_spinners_are_left() {
        use std::path::Path;
        // Split, so this file doesn't match itself.
        let needle = concat!("animate-", "spin");
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        crate::util::source_scan::rust_files(&src, &mut files);
        assert!(files.len() > 20, "source walk found suspiciously few files");
        let offenders: Vec<String> = files
            .iter()
            .filter(|p| std::fs::read_to_string(p).unwrap().contains(needle))
            .map(|p| p.strip_prefix(&src).unwrap_or(p).display().to_string())
            .collect();
        assert!(
            offenders.is_empty(),
            "use loading_dots::WaveDots or SmallDots instead of a spinner in: {offenders:?}"
        );
    }

    /// `SmallDots` are the pill's dots outside the pill: same dot class, same
    /// gap and height as `.pill-activity`, and always open.
    #[test]
    fn small_dots_match_the_pill() {
        let css = include_str!("../../assets/main.css");
        let rule = |sel: &str| {
            let r = &css[css.find(sel).unwrap_or_else(|| panic!("{sel} rule"))..];
            r[..r.find('}').unwrap()].to_string()
        };
        let small = rule(".small-dots {");
        let pill = rule(".pill-activity {");
        for decl in ["gap: 2px;", "height: 8px;", "align-items: center;"] {
            assert!(pill.contains(decl), "premise: .pill-activity has {decl}");
            assert!(
                small.contains(decl),
                ".small-dots must match the pill: {decl}"
            );
        }
        assert!(!small.contains("width: 0"), ".small-dots is always open");
    }

    /// Seeds as the component derives them, plus edge values.
    fn seeds() -> impl Iterator<Item = u64> {
        (0..400u64)
            .map(|n| (1_758_700_000_000.0 + n as f64 * 7_919.0).to_bits())
            .chain([0, 1, u64::MAX])
    }

    #[test]
    fn a_seed_always_gives_the_same_wave() {
        for seed in seeds() {
            assert_eq!(dot_motions(seed), dot_motions(seed));
        }
    }

    #[test]
    fn different_appearances_get_different_waves() {
        let a = dot_motions(1_758_700_000_000.0f64.to_bits());
        let b = dot_motions(1_758_700_000_250.0f64.to_bits());
        assert_ne!(a, b);
    }

    /// The randomness must not break the ripple: each dot's place on the wave
    /// stays strictly after its left neighbour's.
    #[test]
    fn the_crest_still_travels_left_to_right() {
        for seed in seeds() {
            let m = dot_motions(seed);
            let place = |i: usize| (i as f64 - 10.0) * WAVE_STEP_S + m[i].phase_jitter_s;
            for i in 1..WAVE_DOT_COUNT {
                assert!(
                    place(i) > place(i - 1),
                    "seed {seed}: dot {i} starts before dot {}",
                    i - 1
                );
            }
        }
    }

    #[test]
    fn every_value_stays_in_range() {
        let within = |v: f64, (lo, hi): (f64, f64)| v >= lo && v < hi;
        for seed in seeds() {
            for d in dot_motions(seed) {
                assert!(within(d.amp_px, AMP_PX), "{d:?}");
                assert!(d.phase_jitter_s.abs() <= PHASE_JITTER_S, "{d:?}");
                assert!(within(d.swell_px, SWELL_PX), "{d:?}");
                assert!(within(d.swell_period_s, SWELL_PERIOD_S), "{d:?}");
                assert!(
                    d.swell_delay_s <= 0.0 && d.swell_delay_s > -d.swell_period_s,
                    "{d:?}"
                );
                assert!(WAVE_EASES.contains(&d.ease), "{d:?}");
            }
        }
    }

    /// The row's edges are what the 12 px clearances are measured from, so no
    /// dot may leave it.
    #[test]
    fn the_motion_fits_inside_the_row() {
        let reach = AMP_PX.1 + SWELL_PX.1 + DOT_RADIUS_PX;
        assert!(
            reach <= ROW_HEIGHT_PX / 2.0,
            "a dot can reach {reach}px from centre, outside a {ROW_HEIGHT_PX}px row"
        );
        let css = include_str!("../../assets/main.css");
        let row = &css[css.find(".river-flow {").expect(".river-flow rule")..];
        let row = &row[..row.find('}').unwrap()];
        assert!(
            row.contains(&format!("height: {ROW_HEIGHT_PX}px")),
            ".river-flow's height in main.css must equal ROW_HEIGHT_PX"
        );
    }

    #[test]
    fn wave_step_matches_the_css() {
        let css = include_str!("../../assets/main.css");
        assert!(css.contains(&format!("(var(--i) - 10) * {WAVE_STEP_S}s")));
    }

    /// The pill's open dots span must be exactly as wide as its dots (3 px
    /// each, 2 px gaps): narrower clips the last dot, wider leaves a gap that
    /// pushes the label further than the dots need.
    #[test]
    fn pill_width_fits_the_dots() {
        let css = include_str!("../../assets/main.css");
        let rule = &css[css
            .find(".pill-activity[data-active=\"true\"] {")
            .expect("open .pill-activity rule")..];
        let rule = &rule[..rule.find('}').unwrap()];
        let width = SMALL_DOT_COUNT * 3 + (SMALL_DOT_COUNT - 1) * 2;
        assert!(
            rule.contains(&format!("width: {width}px;")),
            "the open .pill-activity width must fit {SMALL_DOT_COUNT} dots ({width}px)"
        );
    }

    /// "They should remain curves": monotone cubic-beziers only, no steps,
    /// no linear, no overshoot.
    #[test]
    fn every_ease_is_a_smooth_monotone_curve() {
        for ease in WAVE_EASES {
            let inner = ease
                .strip_prefix("cubic-bezier(")
                .and_then(|r| r.strip_suffix(')'))
                .unwrap_or_else(|| panic!("{ease} is not a cubic-bezier"));
            let p: Vec<f64> = inner
                .split(',')
                .map(|v| v.trim().parse().unwrap())
                .collect();
            assert_eq!(p.len(), 4, "{ease}");
            assert!(
                (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[2]),
                "{ease}"
            );
            assert_eq!((p[1], p[3]), (0.0, 1.0), "{ease} can overshoot");
            assert!(p[0] > 0.0 && p[2] < 1.0, "{ease} is linear at an end");
        }
    }

    #[test]
    fn style_carries_every_custom_property() {
        let style = dot_motions(42)[3].style(3);
        for prop in [
            "--i: 3;",
            "--amp:",
            "--jitter:",
            "--ease: cubic-bezier(",
            "--swell:",
            "--swell-dur:",
            "--swell-delay: -",
        ] {
            assert!(style.contains(prop), "{prop} missing from {style}");
        }
    }
}
