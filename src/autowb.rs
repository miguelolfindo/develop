//! Auto white balance: find the temperature (and tint) that makes a photo look right, by
//! measuring renders of it, the same way for the Auto button and the MCP server's
//! `auto_white_balance` tool.
//!
//! With faces, skin decides (see skin.rs): skin is measured as if the faces were at 20%
//! brightness (an exposure shift for the measurement only), and the temperature is moved until
//! skin saturation is at `skin::SKIN_SATURATION`, but not so far that the hue drops more than
//! `PINK_LIMIT` below the skin line. Only temperature changes; exposure and tint are kept.
//! Without faces, the low-colour (neutral) areas decide: temperature and tint are stepped until
//! they read neutral. With neither, nothing changes.
//!
//! The search always starts from the camera's own (as-shot) white balance, so running it again
//! gives the same answer whatever the sliders were. Temperature is searched in mired
//! (1,000,000 / kelvin), where equal steps look equal.

use crate::color::{KELVIN_MAX, KELVIN_MIN};

/// Skin hue more than this below the skin line is turning pink or cool.
pub const PINK_LIMIT: f32 = -6.0;
/// How far the skin search may move from where it started, in mired: firelight can need a
/// lot of cooling (5400 K to 2700 K is about 190 mired), warming rarely more than a little.
const MAX_COOLER: f32 = 200.0;
const MAX_WARMER: f32 = 60.0;

/// What one render of the photo measures.
#[derive(Clone, Copy, Debug, Default)]
pub struct Probe {
    pub skin: Option<crate::skin::Reading>,
    /// Neutral areas: blue minus red (positive = too cool), green minus magenta (positive = too
    /// green), and their share of the picture in percent.
    pub blue_minus_red: f32,
    pub green_minus_magenta: f32,
    pub neutral_share: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
    Skin,
    Neutral,
    Unchanged,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub kelvin: f32,
    pub tint: f32,
    pub method: Method,
    pub before: Probe,
    pub after: Probe,
    /// Worth a look, and why.
    pub check: Vec<String>,
    pub renders: usize,
}

fn mired(k: f32) -> f32 {
    1e6 / k
}

fn kelvin(m: f32) -> f32 {
    ((1e6 / m).clamp(KELVIN_MIN, KELVIN_MAX) / 10.0).round() * 10.0
}

/// Search from `start_kelvin` / `start_tint`. `probe(kelvin, tint, exposure_shift)` renders the
/// photo with those values (the shift is for measuring only) and measures it.
pub fn solve(start_kelvin: f32, start_tint: f32, mut probe: impl FnMut(f32, f32, f32) -> Probe) -> Outcome {
    let mut renders = 0;
    let mut shift = 0.0f32;
    // A probe with the faces brought to 20% (measuring shift kept between calls, refined once).
    let mut skin_probe = |k: f32, t: f32, shift: &mut f32, renders: &mut usize| -> Probe {
        let mut p = probe(k, t, *shift);
        *renders += 1;
        if let Some(s) = p.skin {
            let step = crate::skin::stops_to_target(&s);
            if step.abs() > 0.1 {
                *shift = (*shift + step).clamp(-5.0, 5.0);
                p = probe(k, t, *shift);
                *renders += 1;
            }
        }
        p
    };
    let before = skin_probe(start_kelvin, start_tint, &mut shift, &mut renders);
    let mut check = Vec::new();

    // Skin first; if it cannot be solved (often a false face), the neutral areas get a turn.
    'skin: {
    let Some(skin) = before.skin else { break 'skin };
        let target = crate::skin::SKIN_SATURATION;
        // Too strong (too warm): cool down; pale and cool: warm up.
        let error = |p: &Probe| -> Option<f32> {
            let s = p.skin?;
            // Past the pink limit counts as "too cool", whatever the saturation says.
            Some(if s.off < PINK_LIMIT { -1.0 } else { s.saturation - target })
        };
        let e0 = skin.saturation - target;
        let pale_and_cool = skin.off < 0.0 && e0 < -0.08;
        if skin.off >= PINK_LIMIT && e0.abs() <= 0.03 || (e0 < 0.0 && !pale_and_cool && skin.off >= PINK_LIMIT) {
            return Outcome { kelvin: start_kelvin, tint: start_tint, method: Method::Skin, before, after: before, check, renders };
        }
        // Step in mired until the error changes sign, then halve the bracket.
        let direction = if error(&before).unwrap_or(0.0) > 0.0 { 1.0 } else { -1.0 }; // + mired = cooler
        let start = mired(start_kelvin);
        let (limit_warm, limit_cool) = ((start - MAX_WARMER).max(mired(KELVIN_MAX)), (start + MAX_COOLER).min(mired(KELVIN_MIN)));
        let (mut lo, mut lo_err) = (start, error(&before).unwrap_or(0.0));
        let mut hi = lo;
        let mut hi_err = lo_err;
        let mut found = false;
        for _ in 0..12 {
            hi = (hi + direction * 25.0).clamp(limit_warm, limit_cool);
            let p = skin_probe(kelvin(hi), start_tint, &mut shift, &mut renders);
            match error(&p) {
                Some(e) => hi_err = e,
                None => break, // faces lost at this temperature
            }
            if hi_err.signum() != lo_err.signum() {
                found = true;
                break;
            }
            lo = hi;
            lo_err = hi_err;
            if hi <= limit_warm || hi >= limit_cool {
                break;
            }
        }
        let best = if found {
            for _ in 0..5 {
                let mid = 0.5 * (lo + hi);
                let p = skin_probe(kelvin(mid), start_tint, &mut shift, &mut renders);
                let e = error(&p).unwrap_or(hi_err);
                if e.signum() == lo_err.signum() {
                    lo = mid;
                    lo_err = e;
                } else {
                    hi = mid;
                    hi_err = e;
                }
            }
            // The side that is not past the pink limit, nearest the target.
            if lo_err.abs() <= hi_err.abs() && lo_err > -1.0 || hi_err <= -1.0 { lo } else { hi }
        } else {
            // No answer within a plausible distance: probably not real skin (a false face) or a
            // very unusual light. Go by the neutral areas if there are some, else leave it.
            if before.neutral_share >= 3.0 {
                check.push("the skin reading could not be used (perhaps not a real face); went by the neutral areas".into());
                break 'skin;
            }
            check.push("skin could not be brought to the natural range; left as it was (perhaps not a real face)".into());
            let after = skin_probe(start_kelvin, start_tint, &mut shift, &mut renders);
            return Outcome { kelvin: start_kelvin, tint: start_tint, method: Method::Unchanged, before, after, check, renders };
        };
        let k = kelvin(best);
        let after = skin_probe(k, start_tint, &mut shift, &mut renders);
        if let Some(s) = after.skin {
            if s.faces <= 1 && s.brightness < 0.02 {
                check.push("only a small or dark face to go by".into());
            }
        }
        if after.neutral_share >= 5.0 && after.blue_minus_red.abs() > 4.0 {
            check.push(format!(
                "the neutral areas disagree ({:+.1} blue minus red): mixed light, skin was favoured",
                after.blue_minus_red
            ));
        }
        return Outcome { kelvin: k, tint: start_tint, method: Method::Skin, before, after, check, renders };
    }

    if before.neutral_share >= 1.0 {
        let (mut k, mut t) = (start_kelvin, start_tint);
        let mut p = before;
        for _ in 0..6 {
            if p.blue_minus_red.abs() <= 0.8 && p.green_minus_magenta.abs() <= 0.8 {
                break;
            }
            // About 3.5 mired and 3 of tint per level of cast (measured on real photos).
            k = kelvin(mired(k) - 3.5 * p.blue_minus_red);
            t = (t + 3.0 * p.green_minus_magenta).clamp(-150.0, 150.0);
            p = probe(k, t, 0.0);
            renders += 1;
            if p.neutral_share < 0.5 {
                check.push("the neutral areas faded while correcting: a strongly coloured scene".into());
                break;
            }
        }
        if before.neutral_share < 3.0 {
            check.push(format!("only {:.1}% of the picture is neutral to go by", before.neutral_share));
        }
        return Outcome { kelvin: k, tint: t, method: Method::Neutral, before, after: p, check, renders };
    }

    check.push("no faces and nothing neutral to measure: left as it is".into());
    Outcome { kelvin: start_kelvin, tint: start_tint, method: Method::Unchanged, before, after: before, check, renders }
}

impl Outcome {
    pub fn to_json(&self) -> serde_json::Value {
        let probe = |p: &Probe| {
            let mut v = serde_json::json!({
                "neutral_blue_minus_red": (p.blue_minus_red * 10.0).round() / 10.0,
                "neutral_green_minus_magenta": (p.green_minus_magenta * 10.0).round() / 10.0,
                "neutral_share_pct": (p.neutral_share * 10.0).round() / 10.0,
            });
            if let Some(s) = p.skin {
                v["skin_saturation"] = serde_json::json!((s.saturation * 100.0).round() / 100.0);
                v["skin_off_line_deg"] = serde_json::json!((s.off * 10.0).round() / 10.0);
                v["faces"] = serde_json::json!(s.faces);
            }
            v
        };
        serde_json::json!({
            "temperature": self.kelvin,
            "tint": (self.tint * 10.0).round() / 10.0,
            "method": match self.method { Method::Skin => "skin", Method::Neutral => "neutral areas", Method::Unchanged => "unchanged" },
            "before": probe(&self.before),
            "after": probe(&self.after),
            "check": self.check,
            "renders": self.renders,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin::Reading;

    /// A simulated firelit portrait: skin saturation falls as the picture cools, and the hue
    /// turns pink below a certain temperature.
    fn firelit(k: f32, _t: f32, _shift: f32) -> Probe {
        let m = mired(k);
        let saturation = 0.53 - (m - mired(5380.0)) * 0.0017;
        let off = 5.6 - (m - mired(5380.0)) * 0.035;
        Probe { skin: Some(Reading { faces: 3, hue: 123.0 + off, off, saturation, brightness: 0.2 }), ..Default::default() }
    }

    #[test]
    fn skin_brings_firelight_down_to_natural() {
        let out = solve(5380.0, 0.0, firelit);
        assert_eq!(out.method, Method::Skin);
        let s = out.after.skin.unwrap();
        assert!((s.saturation - crate::skin::SKIN_SATURATION).abs() < 0.02, "{s:?} at {}", out.kelvin);
        assert!(out.kelvin < 4000.0 && out.kelvin > 2800.0, "{}", out.kelvin);
        assert_eq!(out.tint, 0.0);
    }

    #[test]
    fn a_false_face_that_never_settles_changes_nothing() {
        // Hue stays far below the pink limit whatever the temperature: no answer.
        let probe = |_k: f32, _t: f32, _s: f32| Probe { skin: Some(Reading { faces: 1, hue: 110.0, off: -13.0, saturation: 0.18, brightness: 0.2 }), ..Default::default() };
        let out = solve(6530.0, 0.0, probe);
        assert_eq!(out.kelvin, 6530.0);
        // (no neutral areas either, so nothing changes)
        assert_eq!(out.method, Method::Unchanged);
        assert!(!out.check.is_empty());
    }

    #[test]
    fn a_false_face_in_a_neutral_scene_goes_by_the_neutral_areas() {
        let probe = |k: f32, t: f32, _s: f32| Probe {
            skin: Some(Reading { faces: 1, hue: 110.0, off: -13.0, saturation: 0.07, brightness: 0.2 }),
            blue_minus_red: 2.0 - (mired(6530.0) - mired(k)) / 3.5,
            green_minus_magenta: -t / 3.0,
            neutral_share: 70.0,
        };
        let out = solve(6530.0, 0.0, probe);
        assert_eq!(out.method, Method::Neutral);
        assert!(out.kelvin > 6530.0);
    }

    #[test]
    fn natural_skin_is_left_alone() {
        let probe = |_k: f32, _t: f32, _s: f32| Probe { skin: Some(Reading { faces: 2, hue: 123.3, off: 0.3, saturation: 0.35, brightness: 0.21 }), ..Default::default() };
        let out = solve(5580.0, 0.0, probe);
        assert_eq!(out.kelvin, 5580.0);
    }

    #[test]
    fn neutral_areas_are_made_neutral() {
        // Neutral reads +3.4 blue at 6530 K and moves 1 level per 3.5 mired; tint +2 green.
        let probe = |k: f32, t: f32, _s: f32| Probe {
            skin: None,
            blue_minus_red: 3.4 - (mired(6530.0) - mired(k)) / 3.5,
            green_minus_magenta: 2.0 - t / 3.0,
            neutral_share: 40.0,
        };
        let out = solve(6530.0, 0.0, probe);
        assert_eq!(out.method, Method::Neutral);
        assert!(out.after.blue_minus_red.abs() <= 0.8 && out.after.green_minus_magenta.abs() <= 0.8, "{:?}", out.after);
        assert!(out.kelvin > 6530.0);
    }

    #[test]
    fn nothing_to_measure_changes_nothing() {
        let out = solve(5000.0, 3.0, |_k, _t, _s| Probe::default());
        assert_eq!(out.method, Method::Unchanged);
        assert_eq!((out.kelvin, out.tint), (5000.0, 3.0));
    }
}
