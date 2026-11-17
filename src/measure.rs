//! Measurements of a rendered photo, so an assistant that cannot see pictures can still judge
//! an edit with numbers: histogram, clipping, brightness zones, contrast, colour cast in the
//! areas that should be neutral, saturation and colours, brightness across the frame, and a
//! detail (sharpness) score. Levels are 0-255 on the sRGB picture as displayed and exported.

use serde_json::{json, Value};

/// The Color Mixer's colours and their centres in on-screen hue (degrees), as in shader.wgsl.
const BANDS: [(&str, f32); 8] = [("red", 352.0), ("orange", 21.0), ("yellow", 49.0), ("green", 100.0), ("aqua", 155.0), ("blue", 210.0), ("purple", 280.0), ("magenta", 320.0)];

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Hue (degrees), saturation and value (0..1) of an 8-bit colour.
fn hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if max <= 0.0 || d <= 0.0 {
        return (0.0, 0.0, max / 255.0);
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, d / max, max / 255.0)
}

/// Value at fraction `q` (0..1) of a 256-bin histogram.
fn percentile(hist: &[u64; 256], total: u64, q: f64) -> u32 {
    let target = (q * total as f64).ceil().max(1.0) as u64;
    let mut seen = 0;
    for (i, &n) in hist.iter().enumerate() {
        seen += n;
        if seen >= target {
            return i as u32;
        }
    }
    255
}

/// Everything measured on an interleaved RGB picture, `width` x `height`.
pub fn measure(rgb: &[u8], width: usize, height: usize) -> Value {
    let total = (width * height).max(1) as u64;
    let pct = |n: u64| round2(n as f64 * 100.0 / total as f64);
    let mut luma_hist = [0u64; 256];
    let mut chan_hist = [[0u64; 256]; 3];
    let mut luma = vec![0f32; width * height];
    let (mut blown, mut any_clip, mut crushed, mut any_black) = (0u64, 0u64, 0u64, 0u64);
    let mut chan_clip = [0u64; 3];
    let mut zones = [0u64; 5];
    let (mut neutral_n, mut neutral_temp, mut neutral_tint) = (0u64, 0f64, 0f64);
    // The whole picture's average colour balance (mid-tones, nothing clipped): a rough backup
    // for when a strong cast leaves no grey areas to measure.
    let (mut whole_n, mut whole_temp, mut whole_tint) = (0u64, 0f64, 0f64);
    let (mut sat_sum, mut very_saturated) = (0f64, 0u64);
    let mut band_n = [0u64; 8];
    let mut band_sat = [0f64; 8];
    let mut band_luma = [0f64; 8];
    let mut grid = [[(0f64, 0u64); 3]; 3];
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 3;
            let (r, g, b) = (rgb[i], rgb[i + 1], rgb[i + 2]);
            let (rf, gf, bf) = (r as f32, g as f32, b as f32);
            let l = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
            luma[y * width + x] = l;
            let li = (l.round() as usize).min(255);
            luma_hist[li] += 1;
            for (c, v) in [r, g, b].into_iter().enumerate() {
                chan_hist[c][v as usize] += 1;
                if v >= 254 {
                    chan_clip[c] += 1;
                }
            }
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            if (40.0..=220.0).contains(&l) && hi < 250 {
                whole_n += 1;
                whole_temp += ((bf - rf) / 2.0) as f64;
                whole_tint += (gf - (rf + bf) / 2.0) as f64;
            }
            if lo >= 254 {
                blown += 1;
            }
            if hi >= 254 {
                any_clip += 1;
            }
            if hi <= 2 {
                crushed += 1;
            }
            if lo <= 1 {
                any_black += 1;
            }
            zones[match li {
                0..=24 => 0,
                25..=84 => 1,
                85..=169 => 2,
                170..=229 => 3,
                _ => 4,
            }] += 1;
            let (h, s, v) = hsv(rf, gf, bf);
            sat_sum += s as f64;
            if s > 0.85 && v > 0.2 {
                very_saturated += 1;
            }
            // Neutral areas: little colour, not too dark or bright to judge.
            if s < 0.10 && (30.0..=235.0).contains(&l) {
                neutral_n += 1;
                neutral_temp += ((bf - rf) / 2.0) as f64;
                neutral_tint += (gf - (rf + bf) / 2.0) as f64;
            }
            if s > 0.15 && v > 0.1 {
                let band = BANDS
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        let d = |c: f32| ((h - c + 540.0).rem_euclid(360.0) - 180.0).abs();
                        d(a.1 .1).partial_cmp(&d(b.1 .1)).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map_or(0, |(k, _)| k);
                band_n[band] += 1;
                band_sat[band] += s as f64;
                band_luma[band] += l as f64;
            }
            let cell = &mut grid[(y * 3 / height.max(1)).min(2)][(x * 3 / width.max(1)).min(2)];
            cell.0 += l as f64;
            cell.1 += 1;
        }
    }

    // Detail: average size of the local brightness change (a Laplacian), on this render's size.
    let mut detail = 0f64;
    let mut detail_n = 0u64;
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let c = luma[y * width + x];
            let lap = 4.0 * c - luma[y * width + x - 1] - luma[y * width + x + 1] - luma[(y - 1) * width + x] - luma[(y + 1) * width + x];
            detail += lap.abs() as f64;
            detail_n += 1;
        }
    }
    let detail = round2(detail / detail_n.max(1) as f64);

    let mean_of = |hist: &[u64; 256]| round1(hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum::<f64>() / total as f64);
    let stats = |hist: &[u64; 256]| {
        json!({
            "mean": mean_of(hist),
            "p1": percentile(hist, total, 0.01), "p5": percentile(hist, total, 0.05),
            "p25": percentile(hist, total, 0.25), "median": percentile(hist, total, 0.5),
            "p75": percentile(hist, total, 0.75), "p95": percentile(hist, total, 0.95),
            "p99": percentile(hist, total, 0.99),
        })
    };
    let luma_mean = mean_of(&luma_hist);
    let luma_sd = {
        let m = luma_mean;
        round1((luma_hist.iter().enumerate().map(|(i, &n)| (i as f64 - m).powi(2) * n as f64).sum::<f64>() / total as f64).sqrt())
    };
    let (p5, p95) = (percentile(&luma_hist, total, 0.05), percentile(&luma_hist, total, 0.95));
    let bins32: Vec<f64> = (0..32).map(|k| pct(luma_hist[k * 8..k * 8 + 8].iter().sum())).collect();
    let grid_means: Vec<Vec<f64>> = grid.iter().map(|row| row.iter().map(|(s, n)| round1(s / (*n).max(1) as f64)).collect()).collect();
    let colours: serde_json::Map<String, Value> = BANDS
        .iter()
        .enumerate()
        .filter(|(k, _)| band_n[*k] > 0)
        .map(|(k, (name, _))| {
            let n = band_n[k] as f64;
            (name.to_string(), json!({"share_percent": pct(band_n[k]), "mean_saturation": round2(band_sat[k] / n), "mean_level": round1(band_luma[k] / n)}))
        })
        .collect();
    let neutral_share = pct(neutral_n);
    let (temp_axis, tint_axis) = if neutral_n > 0 { (round1(neutral_temp / neutral_n as f64), round1(neutral_tint / neutral_n as f64)) } else { (0.0, 0.0) };
    let (whole_temp, whole_tint) = if whole_n > 0 { (round1(whole_temp / whole_n as f64), round1(whole_tint / whole_n as f64)) } else { (0.0, 0.0) };

    // Plain-language reading, for assistants that do better with words than numbers.
    let mut notes: Vec<String> = Vec::new();
    let blown_p = pct(blown);
    if blown_p > 0.5 {
        notes.push(format!("{blown_p}% of the picture is pure white (blown out). Unless that is the sun, a lamp or a reflection, lower Highlights or Whites."));
    }
    let partial = pct(any_clip) - blown_p;
    if partial > 2.0 {
        notes.push(format!("{}% more has one colour channel at maximum (bright colours clipping). Lower Highlights, or Saturation / Vibrance if colours are strong.", round2(partial)));
    }
    let crushed_p = pct(crushed);
    if crushed_p > 1.0 {
        notes.push(format!("{crushed_p}% of the picture is pure black. Raise Shadows or Blacks if detail is wanted there."));
    }
    if luma_mean < 70.0 {
        notes.push(format!("Dark overall: average level {luma_mean} of 255 (a typical daylight scene sits around 100-140). Raise Exposure unless the mood is meant to be dark."));
    } else if luma_mean > 185.0 {
        notes.push(format!("Bright overall: average level {luma_mean} of 255. Lower Exposure unless it is meant to be high key."));
    }
    let range = p95 as i64 - p5 as i64;
    if range < 120 {
        notes.push(format!("Low contrast: 90% of tones fit between levels {p5} and {p95}. Add Contrast, Clarity or Dehaze, or set Whites and Blacks, for more punch."));
    } else if range > 235 {
        notes.push(format!("Very high contrast: tones run from {p5} to {p95}. Lower Contrast or Highlights, or raise Shadows, for a softer result."));
    }
    if neutral_share > 1.0 {
        if temp_axis > 2.5 {
            notes.push(format!("Neutral areas lean blue by {temp_axis} levels: raise Temperature to warm them."));
        } else if temp_axis < -2.5 {
            notes.push(format!("Neutral areas lean yellow / orange by {} levels: lower Temperature to cool them.", -temp_axis));
        }
        if tint_axis > 2.0 {
            notes.push(format!("Neutral areas lean green by {tint_axis} levels: raise Tint (towards magenta)."));
        } else if tint_axis < -2.0 {
            notes.push(format!("Neutral areas lean magenta by {} levels: lower Tint (towards green).", -tint_axis));
        }
    } else if whole_temp.abs() > 15.0 || whole_tint.abs() > 10.0 {
        // No greys left to measure: fall back on the whole picture, if it leans strongly.
        let way = match (whole_temp > 15.0, whole_temp < -15.0) {
            (true, _) => format!("blue by {whole_temp} levels on average: probably too cool, raise Temperature"),
            (_, true) => format!("orange / yellow by {} levels on average: probably too warm, lower Temperature (unless it is a sunset or warm light)", -whole_temp),
            _ if whole_tint > 10.0 => format!("green by {whole_tint} levels on average: raise Tint"),
            _ => format!("magenta by {} levels on average: lower Tint", -whole_tint),
        };
        notes.push(format!("Too few neutral areas to measure the white balance directly, but the whole picture leans {way}."));
    } else {
        notes.push("Too few neutral (grey or white) areas to judge the white balance from the numbers.".into());
    }
    let corners = (grid_means[0][0] + grid_means[0][2] + grid_means[2][0] + grid_means[2][2]) / 4.0;
    if grid_means[1][1] > 40.0 && corners < 0.75 * grid_means[1][1] {
        notes.push(format!("The corners (average level {}) are much darker than the centre ({}): vignetting, natural or added.", round1(corners), grid_means[1][1]));
    }
    let very_sat_p = pct(very_saturated);
    if very_sat_p > 5.0 {
        notes.push(format!("{very_sat_p}% of the picture is very strongly coloured; check it does not look overdone."));
    }

    json!({
        "size": [width, height],
        "brightness": stats(&luma_hist),
        "channels": {"red": stats(&chan_hist[0]), "green": stats(&chan_hist[1]), "blue": stats(&chan_hist[2])},
        "histogram_32_bins_percent": bins32,
        "clipping_percent": {
            "pure_white_all_channels": blown_p,
            "any_channel_at_max": pct(any_clip),
            "red_at_max": pct(chan_clip[0]), "green_at_max": pct(chan_clip[1]), "blue_at_max": pct(chan_clip[2]),
            "pure_black": crushed_p,
            "any_channel_at_zero": pct(any_black),
        },
        "zones_percent": {
            "deep_shadows_0_24": pct(zones[0]), "shadows_25_84": pct(zones[1]), "midtones_85_169": pct(zones[2]),
            "highlights_170_229": pct(zones[3]), "brightest_230_255": pct(zones[4]),
        },
        "contrast": {"std_dev": luma_sd, "range_p5_to_p95": range},
        "neutral_areas": {
            "share_percent": neutral_share,
            "blue_minus_red_half": temp_axis,
            "green_minus_magenta": tint_axis,
            "how_to_read": "Measured on low-colour areas. Positive blue_minus_red_half = too cool (raise temperature), negative = too warm. Positive green_minus_magenta = too green (raise tint), negative = too magenta. Within about ±2.5 is neutral.",
        },
        "whole_picture_balance": {
            "blue_minus_red_half": whole_temp,
            "green_minus_magenta": whole_tint,
            "how_to_read": "Average over all mid-tones. Many scenes lean naturally (sunsets warm, shade cool), so trust neutral_areas first; this helps when there are no grey areas.",
        },
        "saturation": {"mean": round2(sat_sum / total as f64), "very_saturated_percent": very_sat_p},
        "colours": colours,
        "brightness_grid_3x3": grid_means,
        "detail": detail,
        "detail_how_to_read": "Average local contrast at this render size: higher is sharper or more textured. Compare photos rendered at the same size (for focus when culling, or after sharpening, clarity or texture changes).",
        "notes": notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_ramp() {
        let (w, h) = (256usize, 4usize);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| {
            let v = (i % w) as u8;
            [v, v, v]
        }).collect();
        let m = measure(&rgb, w, h);
        assert_eq!(m["brightness"]["median"], json!(127));
        assert!(m["clipping_percent"]["pure_white_all_channels"].as_f64().unwrap() > 0.0);
        assert_eq!(m["neutral_areas"]["blue_minus_red_half"], json!(0.0));
    }

    #[test]
    fn blue_cast_is_noticed() {
        let (w, h) = (64usize, 64usize);
        let rgb: Vec<u8> = (0..w * h).flat_map(|_| [125u8, 128, 134]).collect();
        let m = measure(&rgb, w, h);
        assert!(m["neutral_areas"]["blue_minus_red_half"].as_f64().unwrap() > 2.5);
        assert!(m["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("raise Temperature")));
    }
}
