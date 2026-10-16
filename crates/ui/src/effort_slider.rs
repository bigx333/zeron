//! The reasoning-effort track, after Codex's effort control and Claude's
//! Ultracode treatment (Android's `EffortSlider.kt` is the reference).
//! Effort reads as heat building up: a thick pill whose fill warms from sky
//! to fuchsia as it runs to a white thumb inset in it, a sheen sweeping it,
//! sparks that thicken and quicken (streaking at the high end), a glow that
//! breathes around the fill from Extra High up, and a halo pulsing out of
//! the thumb in the top tiers (`ultra`, `ultracode`, `ultrathink`). The
//! remaining steps show as dots in the empty part of the track.
//!
//! Only the pure geometry, the motion state and the painting live here; the
//! compact model panel owns the data flow (see `pickers::compact`).
use std::sync::OnceLock;
use std::time::Instant;

use gpui::{Bounds, BoxShadow, Corners, Hsla, Pixels, Window, point, px, size};
use zeron_proto::ReasoningLevel;

use crate::motion::{self, MotionSpec};
use crate::theme::Theme;

/// The track's height; the thumb is inset [`THUMB_INSET`] inside it.
pub(crate) const TRACK_HEIGHT: f32 = 28.0;
const THUMB_INSET: f32 = 3.0;
/// The dots marking steps the fill has not reached.
const DOT_RADIUS: f32 = 2.0;
/// Effort as heat: calm sky at the low end, through blue, indigo and
/// violet, to fuchsia at the top.
const HEAT: [(f32, u32); 5] = [
    (0.0, 0x38BDF8),
    (0.35, 0x3B82F6),
    (0.6, 0x6366F1),
    (0.82, 0x8B5CF6),
    (1.0, 0xD946EF),
];
/// The fill shows the band of heat ending at the level, this wide at most,
/// sampled at this many stops.
const HEAT_BAND: f32 = 0.5;
const HEAT_STOPS: usize = 7;
/// The sheen is this many track heights wide.
const SHEEN_WIDTH: f32 = 2.2;
/// Every spark the track can show, and how many show at the lowest step.
const SPARK_COUNT: usize = 44;
const SPARK_MIN: usize = 8;
/// The longest streak behind a spark, at the top step, before its speed.
const STREAK_LENGTH: f32 = 10.0;
/// Sparks fade in over this far inside the fill's rounded start.
const SPARK_EDGE_FADE: f32 = 6.0;
/// The glow's rings around the fill.
const GLOW_RINGS: usize = 5;
const GLOW_RING_STEP: f32 = 2.0;
/// The top tiers' halo: how far it travels out from the thumb, and its stroke.
const HALO_TRAVEL: f32 = 7.0;
const HALO_STROKE: f32 = 1.5;
/// The top tiers fade in and out.
pub(crate) const TOP_FADE: MotionSpec = MotionSpec::new(500, motion::EASE_TAILWIND);
/// The thumb's spring: Compose's `StiffnessMediumLow` with a 0.75 damping
/// ratio, so it settles on a step with a hint of overshoot.
const SPRING_STIFFNESS: f32 = 400.0;
const SPRING_DAMPING: f32 = 0.75;

/// Codex's Ultra and Claude's Ultracode and Ultrathink.
pub(crate) fn is_peak_level(level: ReasoningLevel) -> bool {
    matches!(
        level,
        ReasoningLevel::Ultra | ReasoningLevel::Ultracode | ReasoningLevel::Ultrathink
    )
}

/// The same tiers named by an effort option's choice id.
pub(crate) fn is_peak_id(id: &str) -> bool {
    ["ultra", "ultracode", "ultrathink"]
        .iter()
        .any(|peak| id.eq_ignore_ascii_case(peak))
}

/// The heat palette at `at` (0–1), linear between its stops.
pub(crate) fn heat(at: f32) -> Hsla {
    let at = at.clamp(0.0, 1.0);
    let i = HEAT.iter().position(|(stop, _)| *stop >= at).unwrap_or(0).max(1);
    let ((a, from), (b, to)) = (HEAT[i - 1], HEAT[i]);
    let t = if b > a { (at - a) / (b - a) } else { 1.0 };
    motion::mix(gpui::rgb(from).into(), gpui::rgb(to).into(), t)
}

/// The label and icon colour at `fraction`: never lighter than blue, so it
/// reads on a light surface.
pub(crate) fn label_heat(fraction: f32) -> Hsla {
    heat(0.35 + 0.65 * fraction.clamp(0.0, 1.0))
}

/// 0 below `from`, 1 above `to`, smoothstepped between.
pub(crate) fn ramp(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The heat band the fill shows at `fraction`: from half a track of heat
/// below the level up to it.
pub(crate) fn heat_band(fraction: f32) -> (f32, f32) {
    let fraction = fraction.clamp(0.0, 1.0);
    ((fraction - HEAT_BAND).max(0.0), fraction)
}

/// The thumb's 0–1 position for a pointer `x` across a track `width` wide:
/// the thumb's centre runs a half height in from either end.
pub(crate) fn fraction_at(x: f32, width: f32, height: f32) -> f32 {
    let r = height / 2.0;
    ((x - r) / (width - 2.0 * r).max(1.0)).clamp(0.0, 1.0)
}

/// The step nearest a pointer `x`; `None` without steps.
pub(crate) fn step_at(x: f32, width: f32, height: f32, count: usize) -> Option<usize> {
    (count > 0).then(|| (fraction_at(x, width, height) * (count - 1) as f32).round() as usize)
}

/// Where step `step` of `count` sits along the thumb's run.
pub(crate) fn step_fraction(step: usize, count: usize) -> f32 {
    if count > 1 {
        step.min(count - 1) as f32 / (count - 1) as f32
    } else {
        0.0
    }
}

/// The trackpad feedback for the thumb moving from step `from` to `to`:
/// a tick per detent, a firmer knock crossing into the top tiers, nothing
/// when it stays put.
pub(crate) fn detent_feedback(from: usize, to: usize, peaks: &[bool]) -> Option<crate::haptics::Feedback> {
    use crate::haptics::Feedback;
    let peak = |i: usize| peaks.get(i).copied().unwrap_or(false);
    (from != to).then(|| if to > from && peak(to) && !peak(from) { Feedback::Peak } else { Feedback::Step })
}

/// Whether a pointer `x` has run past either end of the thumb's travel (by
/// more than a few pixels, so resting on the last step doesn't count).
pub(crate) fn past_end(x: f32, width: f32, height: f32) -> bool {
    let r = height / 2.0;
    x < r - EDGE_SLACK || x > width - r + EDGE_SLACK
}

/// How far past the end of its travel the pointer goes before the track
/// bumps back.
const EDGE_SLACK: f32 = 6.0;

/// Reset shows only while the shown step differs from the model's default.
pub(crate) fn reset_visible(shown: usize, default: Option<usize>) -> bool {
    default.is_some_and(|default| default != shown)
}

/// How many sparks show at `fraction`: a few at the lowest step, all of them
/// at the top.
pub(crate) fn visible_sparks(fraction: f32) -> usize {
    let fraction = fraction.clamp(0.0, 1.0);
    let shown = SPARK_MIN as f32 + (SPARK_COUNT - SPARK_MIN) as f32 * fraction;
    (shown as usize).min(SPARK_COUNT)
}

/// The streak behind a spark of unit speed at `fraction`: none below the
/// upper levels.
pub(crate) fn streak_length(fraction: f32) -> f32 {
    ramp(0.6, 1.0, fraction) * STREAK_LENGTH
}

/// The glow's strength around the fill: none below about Extra High,
/// breathing with `breath_wave` (0–1) and strongest in the top tiers.
pub(crate) fn glow_strength(fraction: f32, breath_wave: f32, top: f32) -> f32 {
    ramp(0.55, 1.0, fraction) * (0.55 + 0.45 * breath_wave) * (0.7 + 0.3 * top)
}

/// One drifting point of light. Positions are fractions (x of the fill, y of
/// the track); speed scales the drift; twinkle is pulses per breath.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spark {
    x: f32,
    y: f32,
    radius: f32,
    speed: f32,
    phase: f32,
    twinkle: f32,
}

/// The sparks, seeded so they land in the same places on every run.
fn sparks() -> &'static [Spark] {
    static SPARKS: OnceLock<Vec<Spark>> = OnceLock::new();
    SPARKS.get_or_init(|| {
        // SplitMix64: tiny, seedable, and the same on every platform.
        let mut state: u64 = 7;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
        };
        (0..SPARK_COUNT)
            .map(|_| Spark {
                x: next(),
                y: 0.18 + 0.64 * next(),
                radius: 0.6 + 1.0 * next(),
                speed: 0.5 + next(),
                phase: next(),
                twinkle: 1.0 + 2.0 * next(),
            })
            .collect()
    })
}

/// The track's motion, advanced every frame at a pace set by the effort, so
/// a change of level speeds it up or calms it down without a jump. It stays
/// at rest (one still frame) under reduced motion.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct HeatClock {
    /// The sparks' drift, in fill widths.
    drift: f64,
    /// The sheen's sweep across the fill, 0–1.
    sweep: f64,
    /// A slow breath for the glow, twinkle and halo, in cycles.
    breath: f64,
}

impl HeatClock {
    pub(crate) fn advance(&mut self, dt: f32, intensity: f32) {
        let (dt, i) = (dt.max(0.0) as f64, intensity.clamp(0.0, 1.0) as f64);
        // Drift and breath never wrap: sparks scale them by speeds and
        // twinkles that are not whole numbers, so a wrap would jump them.
        // As f64 they stay precise for years of open panel.
        self.drift += dt * (0.03 + 0.17 * i);
        self.sweep = (self.sweep + dt / (3.4 - 2.2 * i)).rem_euclid(1.0);
        self.breath += dt * (0.4 + 0.6 * i);
    }

    /// 0–1, a full slow cycle per breath.
    fn breath_wave(&self) -> f32 {
        0.5 + 0.5 * (std::f64::consts::TAU * self.breath).sin() as f32
    }

    /// The top tiers' halo, 0 at the thumb's edge to 1 fully out.
    fn halo_wave(&self) -> f32 {
        (self.breath * 1.5).fract() as f32
    }
}

/// A spring on the thumb's 0–1 position, solved in closed form so any frame
/// can sample it and a new target keeps the current velocity.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spring {
    from: f32,
    velocity: f32,
    to: f32,
    started: Instant,
}

impl Spring {
    /// Position and velocity at `now`.
    fn at(&self, now: Instant) -> (f32, f32) {
        let t = now.duration_since(self.started).as_secs_f32() / motion::speed_scale().max(0.001);
        let omega = SPRING_STIFFNESS.sqrt();
        let decay = SPRING_DAMPING * omega;
        let damped = omega * (1.0 - SPRING_DAMPING * SPRING_DAMPING).sqrt();
        let d0 = self.from - self.to;
        let b = (self.velocity + decay * d0) / damped;
        let envelope = (-decay * t).exp();
        let (sin, cos) = (damped * t).sin_cos();
        let offset = envelope * (d0 * cos + b * sin);
        let velocity = envelope
            * (self.velocity * cos - (decay * self.velocity + omega * omega * d0) / damped * sin);
        (self.to + offset, velocity)
    }

    /// The thumb's position toward `target`, and whether it is still moving.
    pub(crate) fn sample(
        state: &mut Option<Self>,
        target: f32,
        now: Instant,
        reduced: bool,
    ) -> (f32, bool) {
        let spring = match state {
            Some(spring) if !reduced => spring,
            _ => {
                Self::hold(state, target, now);
                return (target, false);
            }
        };
        if spring.to != target {
            let (position, velocity) = spring.at(now);
            *spring = Self {
                from: position,
                velocity,
                to: target,
                started: now,
            };
        }
        let (position, velocity) = spring.at(now);
        if (position - target).abs() < 0.0005 && velocity.abs() < 0.005 {
            Self::hold(state, target, now);
            return (target, false);
        }
        (position, true)
    }

    /// Pin the thumb at `value` (under the pointer while dragging), at rest.
    pub(crate) fn hold(state: &mut Option<Self>, value: f32, now: Instant) {
        *state = Some(Self {
            from: value,
            velocity: 0.0,
            to: value,
            started: now,
        });
    }
}

/// A 0–1 fade on a [`MotionSpec`], retargetable mid-flight.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fade {
    from: f32,
    to: f32,
    started: Instant,
}

impl Fade {
    pub(crate) fn sample(
        state: &mut Option<Self>,
        target: f32,
        spec: MotionSpec,
        now: Instant,
        reduced: bool,
    ) -> (f32, bool) {
        let value = |fade: &Self| {
            let span = spec.total().mul_f32(motion::speed_scale()).as_secs_f32();
            let raw = if span > 0.0 {
                now.duration_since(fade.started).as_secs_f32() / span
            } else {
                1.0
            };
            motion::lerp(fade.from, fade.to, spec.progress(raw))
        };
        let fade = match state {
            Some(fade) if !reduced => fade,
            _ => {
                *state = Some(Self {
                    from: target,
                    to: target,
                    started: now,
                });
                return (target, false);
            }
        };
        if fade.to != target {
            *fade = Self {
                from: value(fade),
                to: target,
                started: now,
            };
        }
        let current = value(fade);
        (current, (current - target).abs() > 0.001)
    }
}

/// One frame of the track.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Track {
    /// The thumb's 0–1 position (between steps while it moves).
    pub fraction: f32,
    /// Steps on the track.
    pub count: usize,
    /// 0–1 fade into the top tiers.
    pub top: f32,
    pub clock: HeatClock,
    /// Fast mode's breathing accent halo under the thumb, 0–1.
    pub fast_halo: f32,
    /// 0–1 lift of the thumb while it is held.
    pub press: f32,
}

/// Where the fill's capsule (`r` its end radius, `end` its right edge)
/// starts and ends at height `y`.
fn capsule_span(y: f32, r: f32, end: f32) -> (f32, f32) {
    let dy = (y - r).abs().min(r);
    let bulge = (r * r - dy * dy).sqrt();
    (r - bulge, end - r + bulge)
}

/// Paint the track into `bounds` (its full height is the pill's).
pub(crate) fn paint(window: &mut Window, bounds: Bounds<Pixels>, theme: &Theme, track: Track) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let r = height / 2.0;
    let run = (width - 2.0 * r).max(0.0);
    let fraction = track.fraction.clamp(0.0, 1.0);
    let thumb_x = r + fraction * run;
    let fill_end = (thumb_x + r).min(width);
    let clock = track.clock;
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        Bounds::new(
            bounds.origin + point(px(x), px(y)),
            size(px(w.max(0.0)), px(h.max(0.0))),
        )
    };
    let quad = |window: &mut Window, bounds: Bounds<Pixels>, radius: f32, fill: gpui::Background| {
        window.paint_quad(gpui::quad(
            bounds,
            px(radius),
            fill,
            px(0.0),
            gpui::transparent_black(),
            gpui::BorderStyle::default(),
        ));
    };
    let hot = heat(fraction);

    // A glow around the fill from Extra High up, breathing; strongest in
    // the top tiers.
    let glow = glow_strength(fraction, clock.breath_wave(), track.top);
    if glow > 0.01 {
        for k in 1..=GLOW_RINGS {
            let grow = k as f32 * GLOW_RING_STEP;
            quad(
                window,
                rect(-grow, -grow, fill_end + 2.0 * grow, height + 2.0 * grow),
                r + grow,
                hot.opacity(glow * 0.16 / k as f32).into(),
            );
        }
    }

    crate::glass::light_plate(theme, 1.0).paint(window, bounds, r);
    // Steps not yet reached, as dots in the empty track.
    let dot = theme.text_muted.opacity(0.45);
    for step in 0..track.count {
        let x = r + step_fraction(step, track.count) * run;
        if x - DOT_RADIUS > fill_end {
            quad(
                window,
                rect(x - DOT_RADIUS, r - DOT_RADIUS, 2.0 * DOT_RADIUS, 2.0 * DOT_RADIUS),
                DOT_RADIUS,
                dot.into(),
            );
        }
    }

    // A band of the spectrum ending at the level's heat: calm sky at the
    // lowest step, indigo to fuchsia end to end at the top.
    let (from, to) = heat_band(fraction);
    let stops: Vec<f32> = (0..HEAT_STOPS)
        .map(|i| fill_end * i as f32 / (HEAT_STOPS - 1) as f32)
        .collect();
    paint_strips(window, bounds, fill_end, &stops, |x| {
        heat(from + (to - from) * (x / fill_end.max(1.0)))
    });

    // A sheen sweeping through, quicker and brighter with the effort.
    let band = height * SHEEN_WIDTH;
    let sheen_x = -band + clock.sweep as f32 * (fill_end + 2.0 * band);
    let sheen_peak = 0.10 + 0.22 * fraction;
    if sheen_x < fill_end && sheen_x + band > 0.0 {
        let edges = [sheen_x, sheen_x + band / 2.0, sheen_x + band];
        paint_strips(window, bounds, fill_end, &edges, |x| {
            let t = ((x - sheen_x) / band).clamp(0.0, 1.0);
            gpui::white().opacity(sheen_peak * (1.0 - (2.0 * t - 1.0).abs()))
        });
    }

    // Sparks: denser and quicker with the effort, streaking at the high end.
    let streak = streak_length(fraction);
    for spark in &sparks()[..visible_sparks(fraction)] {
        let x = ((spark.x as f64 + clock.drift * spark.speed as f64).fract() as f32) * fill_end;
        let y = spark.y * height;
        let (left, right) = capsule_span(y, r, fill_end);
        if x - spark.radius < left || x + spark.radius > right {
            continue;
        }
        let pulse = (clock.breath * spark.twinkle as f64 + spark.phase as f64).fract() as f32;
        let twinkle = (std::f32::consts::TAU * pulse).sin().abs();
        let alpha = (0.3 + 0.6 * twinkle)
            * (0.7 + 0.3 * fraction)
            * ramp(left, left + SPARK_EDGE_FADE, x - spark.radius);
        let tail = (x - streak * spark.speed).max(left);
        if x - tail > 1.0 {
            let thickness = spark.radius;
            quad(
                window,
                rect(tail, y - thickness / 2.0, x - tail, thickness),
                thickness / 2.0,
                gpui::linear_gradient(
                    90.0,
                    gpui::linear_color_stop(gpui::white().opacity(0.0), 0.0),
                    gpui::linear_color_stop(gpui::white().opacity(0.6 * alpha), 1.0),
                ),
            );
        }
        quad(
            window,
            rect(x - spark.radius, y - spark.radius, 2.0 * spark.radius, 2.0 * spark.radius),
            spark.radius,
            gpui::white().opacity(alpha).into(),
        );
    }

    let radius = (r - THUMB_INSET).max(0.0);
    // The top tiers: a halo pulsing out from the thumb.
    if track.top > 0.01 {
        let wave = clock.halo_wave();
        // The stroke is drawn inside the quad, so centre it on the ring.
        let ring = radius + wave * HALO_TRAVEL + HALO_STROKE / 2.0;
        window.paint_quad(gpui::quad(
            rect(thumb_x - ring, r - ring, 2.0 * ring, 2.0 * ring),
            px(ring),
            gpui::transparent_black(),
            px(HALO_STROKE),
            hot.opacity(track.top * 0.6 * (1.0 - wave)),
            gpui::BorderStyle::default(),
        ));
    }

    // The thumb: white, inset in the fill, lifting while held and haloed
    // in the accent while fast mode breathes.
    let thumb = rect(thumb_x - radius, r - radius, 2.0 * radius, 2.0 * radius);
    let corners = Corners::all(px(radius));
    let mut drops = Vec::new();
    if track.fast_halo > 0.001 {
        drops.push(BoxShadow {
            color: theme.accent.opacity(0.35 * track.fast_halo),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(6.0),
            spread_radius: px(0.0),
            inset: false,
        });
    }
    if track.press > 0.001 {
        drops.push(BoxShadow {
            color: gpui::black().opacity(0.12 * track.press),
            offset: point(px(0.0), px(2.0 * track.press)),
            blur_radius: px(5.0),
            spread_radius: px(0.0),
            inset: false,
        });
    }
    if !drops.is_empty() {
        window.paint_drop_shadows(thumb, corners, &drops);
    }
    crate::glass::thumb_plate(theme).paint(window, thumb, radius);
}

/// The strips that paint a horizontal colour profile across the fill
/// (`0..fill_end`, the track's full height), cut at `breaks`. Each strip is a
/// two-stop gradient of the profile at its edges — exact for a profile
/// linear between the breaks. GPUI caps a corner's radius at half the quad's
/// width, so the end strips, which carry the fill's rounding, are a full
/// height (two end radii) wide; a fill too short for both is one capsule.
fn strip_edges(breaks: &[f32], fill_end: f32, r: f32) -> Vec<f32> {
    let cap = 2.0 * r;
    if fill_end <= 2.0 * cap + 0.05 {
        return vec![0.0, fill_end.max(0.0)];
    }
    let mut edges: Vec<f32> = [0.0, cap, fill_end - cap, fill_end]
        .into_iter()
        .chain(breaks.iter().map(|x| x.clamp(cap, fill_end - cap)))
        .collect();
    edges.sort_by(f32::total_cmp);
    edges.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    edges
}

fn paint_strips(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    fill_end: f32,
    breaks: &[f32],
    color: impl Fn(f32) -> Hsla,
) {
    let height = f32::from(bounds.size.height);
    let r = (height / 2.0).min(fill_end / 2.0);
    let edges = strip_edges(breaks, fill_end, r);
    // Only strips the profile reaches: a sheen partly off the fill paints
    // nothing past its own ends.
    let (first, last) = (
        breaks.iter().copied().fold(f32::INFINITY, f32::min),
        breaks.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    );
    let zero = px(0.0);
    let corner = px(r);
    let end = edges.len().saturating_sub(2);
    for (i, pair) in edges.windows(2).enumerate() {
        let (x0, x1) = (pair[0], pair[1]);
        if x1 - x0 < 0.05 || x1 <= first || x0 >= last {
            continue;
        }
        let (round_left, round_right) = (i == 0, i == end);
        window.paint_quad(gpui::quad(
            Bounds::new(
                bounds.origin + point(px(x0), px(0.0)),
                size(px(x1 - x0), px(height)),
            ),
            Corners {
                top_left: if round_left { corner } else { zero },
                bottom_left: if round_left { corner } else { zero },
                top_right: if round_right { corner } else { zero },
                bottom_right: if round_right { corner } else { zero },
            },
            gpui::linear_gradient(
                90.0,
                gpui::linear_color_stop(color(x0), 0.0),
                gpui::linear_color_stop(color(x1), 1.0),
            ),
            zero,
            gpui::transparent_black(),
            gpui::BorderStyle::default(),
        ));
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn detents_tick_and_knock_into_the_top_tiers() {
        use crate::haptics::Feedback;
        let peaks = [false, false, false, true, true];
        assert_eq!(detent_feedback(1, 1, &peaks), None);
        assert_eq!(detent_feedback(0, 1, &peaks), Some(Feedback::Step));
        assert_eq!(detent_feedback(2, 3, &peaks), Some(Feedback::Peak));
        // Within the top tiers, and on the way down, a plain tick.
        assert_eq!(detent_feedback(3, 4, &peaks), Some(Feedback::Step));
        assert_eq!(detent_feedback(4, 2, &peaks), Some(Feedback::Step));
        // A jump from below straight into them still knocks.
        assert_eq!(detent_feedback(0, 4, &peaks), Some(Feedback::Peak));
    }

    #[test]
    fn only_running_past_the_travel_bumps() {
        // A 200x44 track: the thumb travels 22..178.
        assert!(!past_end(22.0, 200.0, 44.0));
        assert!(!past_end(180.0, 200.0, 44.0));
        assert!(past_end(10.0, 200.0, 44.0));
        assert!(past_end(190.0, 200.0, 44.0));
    }
    use super::*;
    use std::time::Duration;

    fn rgb(color: Hsla) -> [u8; 3] {
        let c = gpui::Rgba::from(color);
        [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8)
    }

    #[test]
    fn top_tiers_are_ultra_ultracode_and_ultrathink() {
        use ReasoningLevel::*;
        let peaks: Vec<_> = [
            Minimal, Low, Medium, High, XHigh, Max, Ultra, Ultracode, Ultrathink,
        ]
        .into_iter()
        .filter(|level| is_peak_level(*level))
        .collect();
        assert_eq!(peaks, [Ultra, Ultracode, Ultrathink]);
        assert!(is_peak_id("ultra") && is_peak_id("Ultracode") && is_peak_id("ultrathink"));
        assert!(!is_peak_id("max") && !is_peak_id("xhigh") && !is_peak_id("ultra-fast"));
    }

    #[test]
    fn heat_runs_sky_to_fuchsia_through_its_stops() {
        assert_eq!(rgb(heat(0.0)), [0x38, 0xBD, 0xF8]);
        assert_eq!(rgb(heat(0.35)), [0x3B, 0x82, 0xF6]);
        assert_eq!(rgb(heat(0.6)), [0x63, 0x66, 0xF1]);
        assert_eq!(rgb(heat(0.82)), [0x8B, 0x5C, 0xF6]);
        assert_eq!(rgb(heat(1.0)), [0xD9, 0x46, 0xEF]);
        assert_eq!(rgb(heat(-1.0)), rgb(heat(0.0)));
        assert_eq!(rgb(heat(2.0)), rgb(heat(1.0)));
        // Linear between stops.
        let mid = rgb(heat(0.91));
        assert!(mid[0] > 0x8B && mid[0] < 0xD9);
        // The label never starts lighter than blue.
        assert_eq!(rgb(label_heat(0.0)), rgb(heat(0.35)));
        assert_eq!(rgb(label_heat(1.0)), rgb(heat(1.0)));
        assert_eq!(heat_band(0.2), (0.0, 0.2));
        assert_eq!(heat_band(1.0), (0.5, 1.0));
    }

    #[test]
    fn pointer_maps_to_the_nearest_step_inside_the_thumb_run() {
        let (width, height) = (230.0, TRACK_HEIGHT);
        let r = height / 2.0;
        // The ends of the run are the thumb's centre at either end.
        assert_eq!(fraction_at(r, width, height), 0.0);
        assert_eq!(fraction_at(width - r, width, height), 1.0);
        assert_eq!(fraction_at(-50.0, width, height), 0.0);
        assert_eq!(fraction_at(width + 50.0, width, height), 1.0);
        for count in 2..=9 {
            for step in 0..count {
                let x = r + step_fraction(step, count) * (width - 2.0 * r);
                assert_eq!(step_at(x, width, height, count), Some(step));
                // Just short of halfway to the next step stays on this one.
                let nudge = 0.45 * (width - 2.0 * r) / (count - 1) as f32;
                assert_eq!(step_at(x + nudge, width, height, count), Some(step));
            }
        }
        assert_eq!(step_at(100.0, width, height, 0), None);
        assert_eq!(step_at(200.0, width, height, 1), Some(0));
        assert_eq!(step_fraction(0, 1), 0.0);
        assert_eq!(step_fraction(9, 4), 1.0);
    }

    #[test]
    fn sparks_streaks_and_glow_build_with_effort() {
        assert_eq!(visible_sparks(0.0), SPARK_MIN);
        assert_eq!(visible_sparks(0.5), 26);
        assert_eq!(visible_sparks(1.0), SPARK_COUNT);
        assert_eq!(visible_sparks(2.0), SPARK_COUNT);
        let mut last = 0;
        for i in 0..=20 {
            let shown = visible_sparks(i as f32 / 20.0);
            assert!(shown >= last);
            last = shown;
        }
        assert_eq!(streak_length(0.6), 0.0);
        assert_eq!(streak_length(1.0), STREAK_LENGTH);
        assert_eq!(glow_strength(0.5, 1.0, 1.0), 0.0);
        assert!((glow_strength(1.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        assert!(glow_strength(1.0, 0.0, 0.0) < glow_strength(1.0, 1.0, 1.0));
        assert!((ramp(0.55, 1.0, 0.775) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_clock_quickens_with_effort_without_jumps() {
        let (mut low, mut high) = (HeatClock::default(), HeatClock::default());
        low.advance(1.0, 0.0);
        high.advance(1.0, 1.0);
        assert!((low.drift - 0.03).abs() < 1e-9 && (high.drift - 0.2).abs() < 1e-9);
        assert!((low.sweep - 1.0 / 3.4).abs() < 1e-9 && (high.sweep - 1.0 / 1.2).abs() < 1e-9);
        assert!((low.breath - 0.4).abs() < 1e-9 && (high.breath - 1.0).abs() < 1e-9);
        // A change of effort changes the pace, not the place.
        let before = high;
        high.advance(0.0, 0.0);
        assert_eq!(high, before);
        // The sweep wraps; the breath keeps counting, so twinkles never jump.
        let mut clock = HeatClock::default();
        clock.advance(3.5, 0.0);
        assert!(clock.sweep < 1.0 && clock.breath > 1.0);
    }

    #[test]
    fn sparks_are_seeded_and_sized_like_android() {
        let all = sparks();
        assert_eq!(all.len(), SPARK_COUNT);
        assert_eq!(all[0], sparks()[0]);
        for spark in all {
            assert!((0.0..1.0).contains(&spark.x));
            assert!((0.18..=0.82).contains(&spark.y));
            assert!((0.6..=1.6).contains(&spark.radius));
            assert!((0.5..=1.5).contains(&spark.speed));
            assert!((1.0..=3.0).contains(&spark.twinkle));
        }
        // Spread out, not clumped by a broken generator.
        let xs: std::collections::BTreeSet<_> =
            all.iter().map(|s| (s.x * 6.0) as usize).collect();
        assert_eq!(xs.len(), 6);
    }

    #[test]
    fn strips_round_only_the_fill_ends_and_stay_inside_it() {
        let r = TRACK_HEIGHT / 2.0;
        // End strips two radii wide, so GPUI rounds them fully; breaks
        // inside them or past the fill fold into the inner strips' range.
        let edges = strip_edges(&[-30.0, 100.0, 300.0], 200.0, r);
        assert_eq!(edges, [0.0, 2.0 * r, 100.0, 200.0 - 2.0 * r, 200.0]);
        // A fill too short for two end strips is one capsule.
        assert_eq!(strip_edges(&[30.0], 2.0 * r, r), [0.0, 2.0 * r]);
        assert_eq!(strip_edges(&[30.0], 4.0 * r, r), [0.0, 4.0 * r]);
        // The capsule pinches at the top and bottom of its rounded ends.
        assert_eq!(capsule_span(r, r, 120.0), (0.0, 120.0));
        // 2pt from the top, a rounded end is inset by its chord's sagitta.
        let (left, right) = capsule_span(2.0, r, 120.0);
        let inset = r - (r * r - (r - 2.0).powi(2)).sqrt();
        assert!((left - inset).abs() < 0.5 && (120.0 - right - inset).abs() < 0.5);
    }

    #[test]
    fn reset_shows_only_off_the_default() {
        assert!(!reset_visible(2, Some(2)));
        assert!(reset_visible(1, Some(2)));
        assert!(!reset_visible(0, None));
    }

    #[test]
    fn spring_settles_on_the_step_with_a_little_overshoot() {
        let start = Instant::now();
        let mut state = None;
        assert_eq!(Spring::sample(&mut state, 0.0, start, false), (0.0, false));
        assert_eq!(Spring::sample(&mut state, 1.0, start, false), (0.0, true));
        let scale = motion::speed_scale();
        let at = |ms: u64| start + Duration::from_millis(ms).mul_f32(scale);
        let mut peak: f32 = 0.0;
        let mut last = 0.0;
        for ms in (10..=400).step_by(10) {
            let (value, _) = Spring::sample(&mut state, 1.0, at(ms), false);
            peak = peak.max(value);
            last = value;
        }
        assert!(peak > 1.0 && peak < 1.06, "overshoot {peak}");
        assert!((last - 1.0).abs() < 0.01);
        assert_eq!(Spring::sample(&mut state, 1.0, at(2000), false), (1.0, false));
        // Retargeting mid-flight keeps the position continuous.
        Spring::sample(&mut state, 0.0, at(2000), false);
        let before = Spring::sample(&mut state, 0.0, at(2050), false).0;
        let after = Spring::sample(&mut state, 0.5, at(2050), false).0;
        assert!((before - after).abs() < 1e-4);
        // Reduced motion snaps.
        assert_eq!(Spring::sample(&mut state, 0.25, at(2060), true), (0.25, false));
    }

    #[test]
    fn top_tier_fades_in_and_snaps_when_reduced() {
        let start = Instant::now();
        let mut state = None;
        assert_eq!(Fade::sample(&mut state, 0.0, TOP_FADE, start, false), (0.0, false));
        let (value, moving) = Fade::sample(&mut state, 1.0, TOP_FADE, start, false);
        assert!(value == 0.0 && moving);
        let end = start + TOP_FADE.total().mul_f32(motion::speed_scale());
        assert_eq!(Fade::sample(&mut state, 1.0, TOP_FADE, end, false), (1.0, false));
        assert_eq!(Fade::sample(&mut state, 0.0, TOP_FADE, end, true), (0.0, false));
    }
}
