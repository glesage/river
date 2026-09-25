//! Ungated loading visuals; callers control their lifetime.

use dioxus::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};


const WAVE_DOT_COUNT: usize = 10;

// Must fit main.css's open `.pill-activity` width.
const SMALL_DOT_COUNT: usize = 5;

/// Keep `seed` stable while mounted to avoid restarting the animation.
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


pub(crate) fn small_dot_spans() -> Element {
    rsx! {
        for i in 0..SMALL_DOT_COUNT {
            span { key: "{i}", class: "pill-activity-dot", style: "--i: {i};" }
        }
    }
}


#[component]
pub fn WaveDots(testid: &'static str) -> Element {

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


// Must match main.css's `.river-flow-dot` animation-delay step.
const WAVE_STEP_S: f64 = 0.15;

const AMP_PX: (f64, f64) = (2.0, 3.0);
// Keep below half of WAVE_STEP_S so the crest travels strictly left to right.
const PHASE_JITTER_S: f64 = 0.045;

const SWELL_PX: (f64, f64) = (0.6, 1.4);
const SWELL_PERIOD_S: (f64, f64) = (2.2, 3.4);
// Match main.css's `.river-flow` height; motion must fit within the row's clearances.
const ROW_HEIGHT_PX: f64 = 14.0;
// Half the CSS dot width at scale: 1.
const DOT_RADIUS_PX: f64 = 2.5;

// Keep y control points at 0 and 1 to prevent overshoot.
const WAVE_EASES: [&str; 6] = [
    "cubic-bezier(0.37, 0, 0.63, 1)",
    "cubic-bezier(0.45, 0, 0.55, 1)",
    "cubic-bezier(0.65, 0, 0.35, 1)",
    "cubic-bezier(0.42, 0, 0.58, 1)",
    "cubic-bezier(0.3, 0, 0.5, 1)",
    "cubic-bezier(0.5, 0, 0.7, 1)",
];


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


fn dot_motions(seed: u64) -> [DotMotion; WAVE_DOT_COUNT] {
    let mut rng = StdRng::seed_from_u64(seed);
    std::array::from_fn(|_| {
        let swell_period_s = rng.gen_range(SWELL_PERIOD_S.0..SWELL_PERIOD_S.1);
        DotMotion {
            amp_px: rng.gen_range(AMP_PX.0..AMP_PX.1),
            phase_jitter_s: rng.gen_range(-PHASE_JITTER_S..PHASE_JITTER_S),
            ease: WAVE_EASES[rng.gen_range(0..WAVE_EASES.len())],
            swell_px: rng.gen_range(SWELL_PX.0..SWELL_PX.1),
            swell_period_s,
            swell_delay_s: -rng.gen_range(0.0..swell_period_s),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::source_scan::css_rule_body;


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


    /// Top-level declarations for `sel`, gathered from every rule whose
    /// selector list names it, so a grouped selector counts for each member.
    /// Rules nested in `@media` blocks are left out.
    fn declarations_for(css: &str, sel: &str) -> String {
        let mut css = css.to_string();
        while let Some(start) = css.find("/*") {
            let end = css[start..].find("*/").expect("unterminated comment") + start + 2;
            css.replace_range(start..end, "");
        }
        let mut out = String::new();
        let mut depth = 0usize;
        let mut from = 0;
        let mut selector = String::new();
        for (i, c) in css.char_indices() {
            match c {
                '{' => {
                    if depth == 0 {
                        selector = css[from..i].trim().to_string();
                        from = i + 1;
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let named = !selector.starts_with('@')
                            && selector.split(',').any(|s| s.trim() == sel);
                        if named {
                            out.push_str(&css[from..i]);
                        }
                        from = i + 1;
                    }
                }
                _ => {}
            }
        }
        assert!(!out.is_empty(), "no top-level rule names {sel}");
        out
    }

    #[test]
    fn small_dots_match_the_pill() {
        let css = include_str!("../../assets/main.css");
        let small = declarations_for(css, ".small-dots");
        let pill = declarations_for(css, ".pill-activity");
        for decl in [
            "display: inline-flex;",
            "align-items: center;",
            "gap: 2px;",
            "height: 8px;",
        ] {
            assert!(pill.contains(decl), "premise: .pill-activity has {decl}");
            assert!(
                small.contains(decl),
                ".small-dots must match the pill: {decl}"
            );
        }
        assert!(!small.contains("width: 0"), ".small-dots is always open");
        assert!(
            small.contains("flex-shrink: 0;"),
            ".small-dots never shrinks"
        );
        assert!(
            !pill.contains("flex-shrink"),
            "the pill's width is animated, not flex-managed"
        );
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


    #[test]
    fn the_motion_fits_inside_the_row() {
        let reach = AMP_PX.1 + SWELL_PX.1 + DOT_RADIUS_PX;
        assert!(
            reach <= ROW_HEIGHT_PX / 2.0,
            "a dot can reach {reach}px from centre, outside a {ROW_HEIGHT_PX}px row"
        );
        let css = include_str!("../../assets/main.css");
        let row = css_rule_body(css, ".river-flow");
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


    #[test]
    fn pill_width_fits_the_dots() {
        let css = include_str!("../../assets/main.css");
        let rule = css_rule_body(css, ".pill-activity[data-active=\"true\"]");
        let width = SMALL_DOT_COUNT * 3 + (SMALL_DOT_COUNT - 1) * 2;
        assert!(
            rule.contains(&format!("width: {width}px;")),
            "the open .pill-activity width must fit {SMALL_DOT_COUNT} dots ({width}px)"
        );
    }


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
