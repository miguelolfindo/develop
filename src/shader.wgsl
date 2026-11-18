// Stage 2: every adjustment runs here, per screen pixel, every frame.
// The texture holds demosaiced *camera-space* linear RGB (0 = black level, 1 = sensor clip).
//
// "Working space" below is linear sRGB for the built-in profiles and linear ProPhoto RGB
// when an Adobe camera profile (.dcp) is active, as the DNG specification requires.

struct Params {
    // Rows of the white-balanced-camera -> working space matrix (w unused).
    cam_to_work_0: vec4<f32>,
    cam_to_work_1: vec4<f32>,
    cam_to_work_2: vec4<f32>,
    // Rows of the working space -> linear sRGB matrix (w unused).
    work_to_srgb_0: vec4<f32>,
    work_to_srgb_1: vec4<f32>,
    work_to_srgb_2: vec4<f32>,
    // xyz = luminance weights of the working space, w = vibrance (-1..+1).
    luma: vec4<f32>,
    // rgb = white-balance gains (from the Kelvin slider, computed on the CPU), w = smallest gain.
    wb: vec4<f32>,
    // x = 2^exposure, y = contrast, z = saturation (-1..+1), w = 1 if we must sRGB-encode the output.
    basic: vec4<f32>,
    // x = whites, y = highlights, z = shadows, w = blacks, each -1..+1.
    tone: vec4<f32>,
    // xy = size of the viewport in image-UV units, zw = image UV at the viewport centre.
    view: vec4<f32>,
    // Display orientation: texture uv = mat2(rot.xy, rot.zw) * display uv + rot_off.xy.
    rot: vec4<f32>,
    // xy = orientation offset, z = contrast applied after the profile curve (1 = none).
    rot_off: vec4<f32>,
    // x = tone curve: 0 none, 1 built-in fitted curve, 2 lookup table.
    // y = saturation applied before the built-in curve. z, w = 1 if the hue/sat map and the
    // look table are present.
    profile: vec4<f32>,
    // xyz = table size (hues, saturations, values), w = 1 if its value axis is sRGB-encoded.
    hue_sat_dims: vec4<f32>,
    look_dims: vec4<f32>,
    // Straighten: x = cos, y = sin of the angle, z = picture width / height.
    straighten: vec4<f32>,
    // Crop rectangle (x0, y0, x1, y1) in the straightened frame.
    crop: vec4<f32>,
    // Sharpening: x = strength, y = radius in image pixels, z = halo limit, w = masking threshold.
    sharpen: vec4<f32>,
    // Noise reduction: x = luminance, y = colour, z = luminance detail, w = luminance contrast;
    // noise2: x = colour detail, y = colour smoothness (all 0..1).
    noise: vec4<f32>,
    noise2: vec4<f32>,
    // y = 1 to mark clipped highlights and crushed blacks (viewport only).
    // x = input mode. 0: demosaiced Bayer raw. 1: linear raw that was already RGB (linear DNG);
    // same pipeline, the file's baseline exposure is folded into `basic.x`. 2: a finished
    // picture (JPEG, PNG, TIFF) decoded to linear sRGB: no camera matrix, no highlight
    // reconstruction and no base curve, so it is not tone-mapped twice.
    input: vec4<f32>,
    // Presence: x texture, y clarity, z dehaze, each -1..1.
    presence: vec4<f32>,
    // Lens correction. lens_dist: x model (0 none, 1 ptlens, 2 poly3, 3 poly5, 4 embedded
    // knots), yzw coefficients. lens_geo: x Hugin-radius scale, y vignetting-radius scale,
    // z manual distortion, w enlargement. lens_str: x profile distortion strength,
    // y profile vignetting strength, z knot count. tca_r / tca_b: v, c, b, manual scale.
    // vig: k1, k2, k3, w model (0 none, 1 pa, 2 embedded). vig2: x manual amount,
    // y midpoint. fringe: purple amount, purple hue, green amount, green hue.
    lens_dist: vec4<f32>,
    lens_geo: vec4<f32>,
    lens_str: vec4<f32>,
    tca_r: vec4<f32>,
    tca_b: vec4<f32>,
    vig: vec4<f32>,
    vig2: vec4<f32>,
    fringe: vec4<f32>,
    // The camera's embedded correction per knot: distortion, red CA, blue CA, vignetting falloff.
    knots: array<vec4<f32>, 16>,
    // The photo's display levels with the tone sliders at zero: x = fraction above level 176,
    // y = 98th percentile, z = median (0..1).
    scene: vec4<f32>,
    // x = 98th percentile of min(R, G, B) on screen (0..1).
    scene2: vec4<f32>,
    // Spot removal: x = number of spots. Per spot, a = target xy, source zw (texture uv);
    // b = radius (fraction of texture height), feather, opacity, 1 = clone / 0 = heal.
    spot_info: vec4<f32>,
    spot_a: array<vec4<f32>, 64>,
    spot_b: array<vec4<f32>, 64>,
    // Per spot: first point in spot_pts and number of points (1 = circle); target bounding box.
    spot_c: array<vec4<f32>, 64>,
    spot_box: array<vec4<f32>, 64>,
    // Stroke points, two per entry (xy, zw).
    spot_pts: array<vec4<f32>, 512>,
    // Per Heal spot with worked-out gains (spot_c.z = 1): grid origin and 1 / spacing, then its
    // atlas slot (uv of node 0, last node index across and down).
    spot_g: array<vec4<f32>, 64>,
    spot_t: array<vec4<f32>, 64>,
    // Tone curve table (256 entries, four per vec4); curve_info.x = 1 when it changes anything.
    curve_lut: array<vec4<f32>, 64>,
    curve_info: vec4<f32>,
    // Color Mixer: rows 0-1 hue, 2-3 saturation, 4-5 luminance, eight colours (-1..1).
    hsl: array<vec4<f32>, 6>,
    hsl_info: vec4<f32>,
    srgb_to_work_0: vec4<f32>,
    srgb_to_work_1: vec4<f32>,
    srgb_to_work_2: vec4<f32>,
    // Black & white: x = 1 when on; grey brightness per colour (-1..1), eight in two vec4.
    mono_info: vec4<f32>,
    mono_mix: array<vec4<f32>, 2>,
};

// The gamma the tone curve's 0..1 scale is on (measured on Lightroom's point curves).
const CURVE_GAMMA: f32 = 2.2;

@group(0) @binding(0) var raw_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var hue_sat_table: texture_3d<f32>;
@group(0) @binding(4) var look_table: texture_3d<f32>;
@group(0) @binding(5) var curve_table: texture_2d<f32>;
// Wraps around in hue, clamps in saturation and value.
@group(0) @binding(6) var table_sampler: sampler;
// Healing gains (log, per colour), a 128 x 128 slot per spot.
@group(0) @binding(7) var heal_atlas: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // 0..1 across the viewport, origin top-left.
    @location(0) view: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One oversized triangle covering the viewport.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.view = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

const REC709 = vec3<f32>(0.2126, 0.7152, 0.0722);
const CURVE_LUT_SIZE = 1024.0;

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    return select(c * 12.92, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308));
}

fn srgb_encode1(x: f32) -> f32 {
    return srgb_encode(vec3<f32>(x)).x;
}

fn srgb_decode1(x: f32) -> f32 {
    return select(x / 12.92, pow((x + 0.055) / 1.055, 2.4), x > 0.04045);
}

// Hue in 0..6, saturation 0..1, value = brightest channel (may exceed 1).
fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let v = max(c.r, max(c.g, c.b));
    let lo = min(c.r, min(c.g, c.b));
    let d = v - lo;
    if (d <= 1e-9 || v <= 1e-9) {
        return vec3<f32>(0.0, 0.0, v);
    }
    var h: f32;
    if (c.r >= v) {
        h = (c.g - c.b) / d;
    } else if (c.g >= v) {
        h = 2.0 + (c.b - c.r) / d;
    } else {
        h = 4.0 + (c.r - c.g) / d;
    }
    return vec3<f32>(h - 6.0 * floor(h / 6.0), d / v, v);
}

fn hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let k = fract(vec3<f32>(hsv.x / 6.0) + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0;
    let ramp = clamp(abs(k - vec3<f32>(3.0)) - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0));
    return hsv.z * mix(vec3<f32>(1.0), ramp, hsv.y);
}

// Tone curve table entry `i` (0..255), and the curve at `x` (0..1), interpolated.
fn curve_entry(i: i32) -> f32 {
    return params.curve_lut[i / 4][i % 4];
}

fn tone_curve(x: f32) -> f32 {
    let f = clamp(x, 0.0, 1.0) * 255.0;
    let i = min(i32(floor(f)), 254);
    return mix(curve_entry(i), curve_entry(i + 1), f - f32(i));
}

// The Color Mixer's colour centres in on-screen hue (sRGB, degrees), fitted to Lightroom's
// references: red, orange, yellow, green, aqua, blue, purple, magenta, then red again.
// (Purple and magenta could not be measured on the reference photo; they sit between blue
// and red.)
fn mixer_centre(k: i32) -> f32 {
    var centres = array<f32, 9>(-8.0, 21.0, 49.0, 100.0, 155.0, 210.0, 280.0, 320.0, 352.0);
    return centres[k];
}

fn mixer_value(kind: i32, k: i32) -> f32 {
    let i = k % 8;
    return params.hsl[kind * 2 + i / 4][i % 4];
}

// Color Mixer (HSL), fitted to Lightroom's references (photo DSC05736, every colour's hue,
// saturation and luminance at -100 and +100). It works on the picture as displayed (after
// the profile's tone curve), so the colours are the ones seen on screen:
// - A colour's on-screen hue sets how much of each of its two nearest colours applies,
//   blended smoothly between their centres; greys are left alone.
// - Hue moves a colour towards the neighbouring colour's centre (90% of the way at 100),
//   keeping most of its brightness.
// - Saturation moves it towards or away from a grey of the same luminance: -100 is grey,
//   +100 roughly doubles a muted colour and stops short of the gamut edge.
// - Luminance brightens or darkens it without changing its hue, more for saturated colours
//   (about 0.75 stop up and 1.1 stops down at 100).
fn color_mixer(work: vec3<f32>) -> vec3<f32> {
    let srgb_raw = vec3<f32>(
        dot(params.work_to_srgb_0.xyz, work),
        dot(params.work_to_srgb_1.xyz, work),
        dot(params.work_to_srgb_2.xyz, work),
    );
    let lin = clamp(srgb_raw, vec3<f32>(0.0), vec3<f32>(1.0));
    let enc = srgb_encode(lin);
    let hsv = rgb_to_hsv(enc);
    var deg = hsv.x * 60.0;
    if (deg >= mixer_centre(8)) {
        deg -= 360.0;
    }
    var k = 0;
    for (var j = 1; j < 8; j++) {
        if (deg >= mixer_centre(j)) {
            k = j;
        }
    }
    let span = mixer_centre(k + 1) - mixer_centre(k);
    let t = smoothstep(0.0, 1.0, (deg - mixer_centre(k)) / span);
    let colourful = smoothstep(0.02, 0.15, hsv.y);
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let y0 = dot(lin, lum);
    var out = lin;
    // Hue: each of the two colours pulls towards its own neighbour.
    var shift = 0.0;
    for (var n = 0; n < 2; n++) {
        let band = k + n;
        let w = select(1.0 - t, t, n == 1);
        let a = mixer_value(0, band);
        let to_next = mixer_centre(band % 8 + 1) - mixer_centre(band % 8);
        let prev = (band + 7) % 8;
        let to_prev = mixer_centre(prev + 1) - mixer_centre(prev);
        shift += w * a * select(to_prev, to_next, a > 0.0);
    }
    if (abs(shift) > 1e-4) {
        var h = hsv.x * 60.0 + 0.9 * shift * colourful;
        h = h - 360.0 * floor(h / 360.0);
        let moved = srgb_decode3(hsv_to_rgb(vec3<f32>(h / 60.0, hsv.y, hsv.z)));
        let y1 = max(dot(moved, lum), 1e-6);
        out = moved * pow(max(y0, 1e-6) / y1, 0.4);
    }
    // Saturation, towards or away from grey of the same luminance.
    let sat = mix(mixer_value(1, k), mixer_value(1, k + 1), t) * colourful;
    if (abs(sat) > 1e-4) {
        let y = dot(out, lum);
        var f = 1.0 + sat;
        if (sat > 0.0) {
            let kk = 1.9 * sat;
            f = (1.0 + kk) / (1.0 + kk * hsv.y);
        }
        out = max(vec3<f32>(y) + (out - vec3<f32>(y)) * f, vec3<f32>(0.0));
    }
    // Luminance.
    let l = mix(mixer_value(2, k), mixer_value(2, k + 1), t);
    if (abs(l) > 1e-4) {
        let strength = pow(clamp(hsv.y / 0.45, 0.0, 1.0), 0.6) * colourful;
        out *= exp2(l * select(1.1, 0.75, l > 0.0) * strength);
    }
    // Back to the working space, keeping anything the clamp above cut off.
    let back = out + (srgb_raw - lin);
    return vec3<f32>(
        dot(params.srgb_to_work_0.xyz, back),
        dot(params.srgb_to_work_1.xyz, back),
        dot(params.srgb_to_work_2.xyz, back),
    );
}

// Black & white: grey at the colour's own luminance, made brighter or darker by how the B&W
// mix sets the colour's band (blended between the two nearest bands as in the Color Mixer).
// Up to about 1.2 stops at 100, more for saturated colours; greys stay as they are.
fn mono_value(k: i32) -> f32 {
    let i = k % 8;
    return params.mono_mix[i / 4][i % 4];
}

fn black_and_white(work: vec3<f32>) -> vec3<f32> {
    let srgb_raw = vec3<f32>(
        dot(params.work_to_srgb_0.xyz, work),
        dot(params.work_to_srgb_1.xyz, work),
        dot(params.work_to_srgb_2.xyz, work),
    );
    let lin = clamp(srgb_raw, vec3<f32>(0.0), vec3<f32>(1.0));
    let hsv = rgb_to_hsv(srgb_encode(lin));
    var deg = hsv.x * 60.0;
    if (deg >= mixer_centre(8)) {
        deg -= 360.0;
    }
    var k = 0;
    for (var j = 1; j < 8; j++) {
        if (deg >= mixer_centre(j)) {
            k = j;
        }
    }
    let t = smoothstep(0.0, 1.0, (deg - mixer_centre(k)) / (mixer_centre(k + 1) - mixer_centre(k)));
    let amount = mix(mono_value(k), mono_value(k + 1), t);
    let strength = pow(clamp(hsv.y / 0.45, 0.0, 1.0), 0.6) * smoothstep(0.02, 0.15, hsv.y);
    let y = dot(srgb_raw, vec3<f32>(0.2126, 0.7152, 0.0722));
    let grey = vec3<f32>(max(y, 0.0) * exp2(1.2 * amount * strength));
    return vec3<f32>(
        dot(params.srgb_to_work_0.xyz, grey),
        dot(params.srgb_to_work_1.xyz, grey),
        dot(params.srgb_to_work_2.xyz, grey),
    );
}

fn srgb_decode3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_decode1(c.r), srgb_decode1(c.g), srgb_decode1(c.b));
}

// Texture coordinate of a colour in a hue/sat/value table (see profile.rs for the layout).
fn table_coord(hsv: vec3<f32>, dims: vec4<f32>) -> vec3<f32> {
    var v = clamp(hsv.z, 0.0, 1.0);
    if (dims.w > 0.5) {
        v = srgb_encode1(v);
    }
    return vec3<f32>(
        (hsv.x / 6.0 * dims.x + 0.5) / dims.x,
        (hsv.y * (dims.y - 1.0) + 0.5) / dims.y,
        (v * (dims.z - 1.0) + 0.5) / dims.z,
    );
}

// Apply one table entry: x = hue shift in degrees, y = saturation scale, z = value scale.
fn table_apply(hsv: vec3<f32>, entry: vec3<f32>, dims: vec4<f32>) -> vec3<f32> {
    let hue = hsv.x + entry.x / 60.0;
    let sat = min(hsv.y * entry.y, 1.0);
    var gain = entry.z;
    if (dims.w > 0.5) {
        // The scale is defined on the sRGB-encoded value; convert it to a linear gain.
        let base = clamp(hsv.z, 1e-6, 1.0);
        gain = srgb_decode1(min(srgb_encode1(base) * entry.z, 1.0)) / base;
    }
    return hsv_to_rgb(vec3<f32>(hue - 6.0 * floor(hue / 6.0), sat, hsv.z * gain));
}

fn curve_lookup(x: f32) -> f32 {
    let u = sqrt(clamp(x, 0.0, 1.0));
    let coord = vec2<f32>((u * (CURVE_LUT_SIZE - 1.0) + 0.5) / CURVE_LUT_SIZE, 0.5);
    return textureSampleLevel(curve_table, texture_sampler, coord, 0.0).r;
}

// Built-in "Standard" curve, fitted to patch measurements of Lightroom's default rendering
// of the reference file: an exponential shoulder times a toe that keeps deep shadows dark.
fn fitted_curve(x: vec3<f32>) -> vec3<f32> {
    let toe = vec3<f32>(0.22) + 0.78 * smoothstep(vec3<f32>(0.0), vec3<f32>(0.20), x);
    return (vec3<f32>(1.0) - exp(-4.4 * x)) * toe;
}

// White balance in camera space, with highlight reconstruction.
// A saturated channel only tells us "at least this bright". Leaving it at its own ceiling is
// what makes blown areas turn pink (green saturates first, so red and blue end up above it).
// Instead, estimate each saturated channel as the average of the other two when that is
// higher: exact for neutral light and close for warm light, where green sits between red and
// blue. Once all three are saturated nothing is known, so the pixel goes to neutral white.
// Unsaturated pixels are untouched, and the result keeps real gradation above 1.0 for
// Exposure / Highlights to pull back.
fn balance(cam: vec3<f32>) -> vec3<f32> {
    let sensor = min(cam, vec3<f32>(1.0));
    let wb_raw = sensor * params.wb.rgb;
    let others = (vec3<f32>(wb_raw.r + wb_raw.g + wb_raw.b) - wb_raw) * 0.5;
    let saturated = smoothstep(vec3<f32>(0.9), vec3<f32>(1.0), sensor);
    let estimated = mix(wb_raw, max(wb_raw, others), saturated);
    let brightest = max(estimated.r, max(estimated.g, estimated.b));
    // Start fading to white well before the last channel saturates, so the blown core has a
    // soft edge instead of a hard disc.
    let all_blown = smoothstep(0.35, 1.0, min(sensor.r, min(sensor.g, sensor.b)));
    return mix(estimated, vec3<f32>(brightest), all_blown) / params.wb.w;
}

fn to_work(balanced: vec3<f32>) -> vec3<f32> {
    return max(vec3<f32>(
        dot(params.cam_to_work_0.xyz, balanced),
        dot(params.cam_to_work_1.xyz, balanced),
        dot(params.cam_to_work_2.xyz, balanced),
    ), vec3<f32>(0.0));
}

// Point `k` of the spot strokes (two are packed in each entry).
fn spot_point(k: i32) -> vec2<f32> {
    let v = params.spot_pts[k / 2];
    return select(v.xy, v.zw, (k & 1) == 1);
}

// Distance from `p` (texture coordinates) to spot `i`'s centre line (a single point for a
// circle), in picture heights.
fn spot_distance(i: i32, p: vec2<f32>, aspect: f32) -> f32 {
    let c = params.spot_c[i];
    let start = i32(c.x);
    let n = i32(c.y);
    var a = spot_point(start);
    var pa = vec2<f32>((p.x - a.x) * aspect, p.y - a.y);
    var best = length(pa);
    for (var k = 1; k < n; k++) {
        let b = spot_point(start + k);
        let ba = vec2<f32>((b.x - a.x) * aspect, b.y - a.y);
        let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
        best = min(best, length(pa - ba * h));
        a = b;
        pa = vec2<f32>((p.x - a.x) * aspect, p.y - a.y);
    }
    return best;
}

// The stored picture at `uv` with the Clone spots before spot `upto` applied: what a later
// spot copies from and, when it heals, measures its surroundings on. So cloning out an
// awkward edge first and healing over it afterwards works as in Lightroom: the heal sees
// the cloned edge, not the original. (Earlier Heal spots are not seen by later spots.)
fn cloned_at(uv: vec2<f32>, lod: f32, upto: i32, aspect: f32) -> vec3<f32> {
    var c = textureSampleLevel(raw_texture, texture_sampler, uv, lod).rgb;
    for (var j = 0; j < upto; j++) {
        let b = params.spot_b[j];
        let box = params.spot_box[j];
        if (b.w < 0.5 || uv.x < box.x || uv.y < box.y || uv.x > box.z || uv.y > box.w) {
            continue;
        }
        let dist = spot_distance(j, uv, aspect) / b.x;
        if (dist >= 1.0) {
            continue;
        }
        let a = params.spot_a[j];
        let mask = (1.0 - smoothstep(1.0 - max(b.y, 0.02), 1.0, dist)) * b.z;
        c = mix(c, textureSampleLevel(raw_texture, texture_sampler, uv + a.zw - a.xy, lod).rgb, mask);
    }
    return c;
}

// Healing, as a stand-in until the processor has worked out the spot's gains across its
// whole shape (spots::HealGrid, normally ready by the next frame): how much brighter (per
// colour) the target's surroundings are than the source's, seen from `uv`. Eight directions are read at 1.35 brush radii, at a level of detail that
// averages over a fair part of the band; directions that land on the spot itself (its
// blemish) count less the deeper they are in, and where most of them do (inside a wide
// painted patch) a second ring further out takes over. The ratios are blended in log space,
// then blended again with readings far from that first blend turned down, so a branch or
// an edge next to the spot does not darken or tint the whole patch. As `uv` moves, the
// readings move with it, so a gradient across the spot is carried smoothly through it.
fn heal_gain(i: i32, uv: vec2<f32>, off: vec2<f32>, r: f32, aspect: f32, height: f32) -> vec3<f32> {
    let lod = max(log2(r * height * 0.5), 0.0);
    var logs: array<vec3<f32>, 16>;
    var ws: array<f32, 16>;
    var count = 0;
    var near = 0.0;
    for (var ring = 0; ring < 2; ring++) {
        // The outer ring only matters where the inner one found little outside the spot.
        let share = select(1.0, clamp(1.0 - near / 2.0, 0.0, 1.0), ring == 1);
        if (share <= 0.0) {
            break;
        }
        let reach = select(1.35, 2.7, ring == 1) * r;
        for (var k = 0; k < 8; k++) {
            let angle = (f32(k) + 0.5 * f32(ring)) * 0.7853982;
            let q = uv + vec2<f32>(cos(angle) / aspect, sin(angle)) * reach;
            let w = share * smoothstep(1.05, 1.3, spot_distance(i, q, aspect) / r);
            if (w <= 0.0) {
                continue;
            }
            let t = cloned_at(q, lod, i, aspect);
            let s = cloned_at(q + off, lod, i, aspect);
            let ratio = clamp((t + vec3<f32>(0.002)) / (s + vec3<f32>(0.002)), vec3<f32>(0.25), vec3<f32>(4.0));
            logs[count] = log(ratio);
            ws[count] = w;
            count++;
            if (ring == 0) {
                near += w;
            }
        }
    }
    var sum = vec3<f32>(0.0);
    var weights = 0.0;
    for (var k = 0; k < count; k++) {
        sum += ws[k] * logs[k];
        weights += ws[k];
    }
    if (weights < 1e-3) {
        return vec3<f32>(1.0);
    }
    // Two rounds of turning down the readings that disagree with the blend.
    for (var round = 0; round < 2; round++) {
        let centre = sum / weights;
        sum = vec3<f32>(0.0);
        weights = 0.0;
        for (var k = 0; k < count; k++) {
            let d = logs[k] - centre;
            let w = ws[k] * exp(-dot(d, d) / 0.02);
            sum += w * logs[k];
            weights += w;
        }
        if (weights < 1e-4) {
            return exp(centre);
        }
    }
    return exp(sum / weights);
}

// The stored picture at `uv`, with the spots healed or cloned in. Everything that reads the
// picture goes through this, so later adjustments (sharpening, clarity, dehaze) see the
// healed picture, not the original underneath.
// Noise reduction.
// Colour: the colour of a wide neighbourhood (a few pixels to several, with the slider),
// brightness kept; neighbours of very different brightness count less, so colour does not
// bleed across edges. Luminance: an edge-preserving average of brightness on a perceptual
// (square-root) scale: neighbours of similar brightness are averaged in, those across an edge
// are not; the reach and the brightness difference counted as noise both grow with the slider.
// Applied as a gain, so colour is untouched by it.
fn reduce_noise(cam_in: vec3<f32>, uv: vec2<f32>, footprint: f32, texels: vec2<f32>) -> vec3<f32> {
    var cam = cam_in;
    let w = vec3<f32>(0.25, 0.5, 0.25);
    let lod = max(log2(footprint), 0.0);
    let colour = params.noise.y;
    if (colour > 0.0) {
        let l = max(dot(cam, w), 1.0e-5);
        // Smoothness: how wide the colour is averaged (blotches need a wide reach). Detail: how
        // strictly neighbours must match in brightness, which keeps small coloured details.
        let smoothness = params.noise2.y;
        let colour_detail = params.noise2.x;
        let clod = lod + 0.5 + (1.0 + 2.5 * smoothness) * colour;
        let tolerance = 0.6 - 0.45 * colour_detail;
        // 5 x 5 samples half the reach apart, each from a copy one size finer: overlapping and
        // weighted to the middle, so the averaged colour changes smoothly, without blocks.
        let cstep = exp2(clod) * 0.5 / texels;
        let slod = max(clod - 1.0, lod);
        var acc = vec3<f32>(0.0);
        var total = 0.0;
        for (var j = -2; j <= 2; j++) {
            for (var i = -2; i <= 2; i++) {
                let s = raw_at(uv + vec2<f32>(f32(i), f32(j)) * cstep, slod);
                let ls = max(dot(s, w), 1.0e-5);
                let d = log(ls / l);
                let weight = exp(-d * d / (2.0 * tolerance * tolerance)) * exp(-f32(i * i + j * j) / 4.5);
                acc += s / ls * weight;
                total += weight;
            }
        }
        let ratio = acc / max(total, 1.0e-6);
        cam = mix(cam, l * ratio, min(colour * 1.25, 1.0));
    }
    let luminance = params.noise.x;
    if (luminance > 0.0) {
        let size = exp2(lod);
        let old_l = max(dot(cam, w), 0.0);
        let c = sqrt(old_l);
        // Specks: a pixel brighter or darker than all eight of its neighbours is noise, not
        // detail (a real edge always has neighbours on both sides of it). Pulled into their range,
        // it no longer stands out, and the averaging below compares against that, not the speck.
        // Specks also come as short streaks of two or three pixels, which are each other's
        // neighbours, so the range is that of the neighbours after the two darkest and two
        // brightest (the 3rd darkest to the 3rd brightest); at low settings, after one.
        let near = size / texels;
        var lo = vec3<f32>(1.0e9);
        var hi = vec3<f32>(-1.0e9);
        for (var j = -1; j <= 1; j++) {
            for (var i = -1; i <= 1; i++) {
                if (i == 0 && j == 0) {
                    continue;
                }
                let t = sqrt(max(dot(raw_at(uv + vec2<f32>(f32(i), f32(j)) * near, lod), w), 0.0));
                // Keep the three lowest (lo.x <= lo.y <= lo.z) and three highest (hi.x >= ...).
                if (t < lo.z) {
                    lo.z = t;
                    if (lo.z < lo.y) { lo = vec3<f32>(lo.x, lo.z, lo.y); }
                    if (lo.y < lo.x) { lo = vec3<f32>(lo.y, lo.x, lo.z); }
                }
                if (t > hi.z) {
                    hi.z = t;
                    if (hi.z > hi.y) { hi = vec3<f32>(hi.x, hi.z, hi.y); }
                    if (hi.y > hi.x) { hi = vec3<f32>(hi.y, hi.x, hi.z); }
                }
            }
        }
        let deep = smoothstep(0.3, 0.8, luminance);
        let floor_ = mix(lo.y, lo.z, deep);
        let ceiling = mix(hi.y, hi.z, deep);
        let centre = mix(c, clamp(c, floor_, ceiling), min(luminance * 2.0, 1.0));
        // Brightness differences up to about this much (on the square-root scale) count as
        // noise: from fine grain at low settings to heavy high-ISO grain at 100; real edges are
        // usually far bigger and are kept.
        // Detail: lower treats bigger differences as noise (smoother); higher keeps more.
        // (Too generous a tolerance flattens whole areas into patches, like watercolour.)
        let sigma = (0.006 + 0.09 * luminance) * (1.5 - params.noise.z);
        // The reach grows with the slider so coarse grain (blotches) is smoothed too, not just
        // fine; each sample from a copy one size finer than the spacing, so it is partly averaged
        // already but still changes smoothly between samples (no blocks).
        let llod = lod + log2(1.0 + 2.0 * luminance);
        let step = exp2(llod) / texels;
        let slod = max(llod - 1.0, lod);
        var sum = centre;
        var total = 1.0;
        for (var j = -2; j <= 2; j++) {
            for (var i = -2; i <= 2; i++) {
                if (i == 0 && j == 0) {
                    continue;
                }
                let t = sqrt(max(dot(raw_at(uv + vec2<f32>(f32(i), f32(j)) * step, slod), w), 0.0));
                let ds = f32(i * i + j * j);
                let weight = exp(-ds / 4.5) * exp(-(t - centre) * (t - centre) / (2.0 * sigma * sigma));
                sum += t * weight;
                total += weight;
            }
        }
        var evened = sum / total;
        // Contrast: give back what the averaging flattened where it is bigger than the noise
        // (texture, local contrast), leaving the small differences (the grain) smoothed.
        let removed = centre - evened;
        let structure = smoothstep(sigma * 0.6, sigma * 2.0, abs(removed));
        evened += removed * structure * params.noise.w;
        // A trace of the fine grain stays even at 100 (without the specks), so smooth areas look
        // like a photograph rather than plastic.
        let aim = mix(centre, evened, min(luminance * 1.5, 1.0) * 0.9);
        let new_l = aim * aim;
        // New brightness, same colour. A pixel that was far too dark has no colour worth keeping
        // (a black speck would stay black however it is scaled), so it takes its surroundings'.
        let around = raw_at(uv, llod + 1.0);
        let around_colour = around / max(dot(around, w), 1.0e-5);
        let own_colour = cam / max(old_l, 1.0e-6);
        let trust = smoothstep(0.05, 0.35, old_l / max(new_l, 1.0e-6));
        cam = mix(around_colour, own_colour, trust) * new_l;
    }
    return cam;
}

fn raw_at(uv: vec2<f32>, lod: f32) -> vec3<f32> {
    var c = textureSampleLevel(raw_texture, texture_sampler, uv, lod).rgb;
    let n = i32(params.spot_info.x);
    if (n == 0) {
        return c;
    }
    let size = vec2<f32>(textureDimensions(raw_texture, 0));
    let aspect = size.x / size.y;
    for (var i = 0; i < n; i++) {
        let box = params.spot_box[i];
        if (uv.x < box.x || uv.y < box.y || uv.x > box.z || uv.y > box.w) {
            continue;
        }
        let a = params.spot_a[i];
        let b = params.spot_b[i];
        let dist = spot_distance(i, uv, aspect) / b.x;
        if (dist >= 1.0) {
            continue;
        }
        let soft = max(b.y, 0.02);
        let mask = (1.0 - smoothstep(1.0 - soft, 1.0, dist)) * b.z;
        let off = a.zw - a.xy;
        var s = cloned_at(uv + off, lod, i, aspect);
        if (b.w < 0.5) {
            if (params.spot_c[i].z > 0.5) {
                // Worked out across the whole shape on the processor (spots::HealGrid).
                let g = params.spot_g[i];
                let t = params.spot_t[i];
                let node = clamp((uv - g.xy) * g.zw, vec2<f32>(0.0), t.zw);
                let atlas = vec2<f32>(textureDimensions(heal_atlas, 0));
                s *= exp(textureSampleLevel(heal_atlas, texture_sampler, t.xy + node / atlas, 0.0).rgb);
            } else {
                s *= heal_gain(i, uv, off, b.x, aspect, size.y);
            }
        }
        c = mix(c, s, mask);
    }
    return c;
}

// Quick brightness of a camera-space sample, good enough to compare neighbours.
fn rough_luma(cam: vec3<f32>) -> f32 {
    return max(dot(balance(cam), vec3<f32>(0.25, 0.5, 0.25)), 1e-5);
}

// One neighbourhood sample: returns (weight * brightness, weight). The weight falls off
// the further the sample's brightness is from the pixel's own (about one stop wide), so a
// dark roof next to bright sky does not drag the sky's average down, and vice versa.
fn local_tap(uv: vec2<f32>, lod: f32, weight: f32, own: f32) -> vec2<f32> {
    let l = rough_luma(raw_at(uv, lod));
    let stops = log2(l / own);
    let w = weight * exp(-0.5 * stops * stops);
    return vec2<f32>(w * l, w);
}

// Brightness of the neighbourhood of `uv` (roughly 3% of the frame) relative to the pixel
// itself: 1 = same, 2 = surroundings twice as bright. Read from the small mip levels of the
// image, which are already blurred copies of it.
fn local_ratio(uv: vec2<f32>, cam: vec3<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(raw_texture, 0));
    let lod = max(log2(max(size.x, size.y)) - 6.4, 0.0);
    let step = 1.5 * exp2(lod) / size;
    let own = rough_luma(cam);
    // The pixel itself gets a small vote so the sum can never be empty.
    var acc = vec2<f32>(0.02 * own, 0.02);
    acc += local_tap(uv, lod, 0.2, own);
    acc += local_tap(uv + step * vec2<f32>(1.0, 0.0), lod, 0.12, own);
    acc += local_tap(uv + step * vec2<f32>(-1.0, 0.0), lod, 0.12, own);
    acc += local_tap(uv + step * vec2<f32>(0.0, 1.0), lod, 0.12, own);
    acc += local_tap(uv + step * vec2<f32>(0.0, -1.0), lod, 0.12, own);
    acc += local_tap(uv + step * vec2<f32>(0.7, 0.7), lod, 0.08, own);
    acc += local_tap(uv + step * vec2<f32>(-0.7, 0.7), lod, 0.08, own);
    acc += local_tap(uv + step * vec2<f32>(0.7, -0.7), lod, 0.08, own);
    acc += local_tap(uv + step * vec2<f32>(-0.7, -0.7), lod, 0.08, own);
    return acc.x / acc.y / own;
}

// Shadows strength from the photo's display levels (see fs_main).
const SHADOW_BASE: f32 = 0.2;
const SHADOW_PER_BRIGHT: f32 = 2.4;
const SHADOW_PER_MEDIAN: f32 = 0.9;

// Whites white-point gain (see fs_main): stops per stop the 98th and 99.7th percentiles sit
// below white, its cap, and how steeply it grows along the slider.
const WHITES_ROOM_P98: f32 = 3.4;
const WHITES_ROOM_P997: f32 = 2.5;
const WHITES_ROOM_MAX: f32 = 2.6;
const WHITES_ROOM_POWER: f32 = 2.6;

// How strongly Highlights adapts to the photo's 98th-percentile display level (see fs_main).
const HIGHLIGHT_ADAPT: f32 = 2.2;
const HL_STRENGTH_BASE: f32 = 2.6;
const HL_STRENGTH_PER_MEDIAN: f32 = 6.0;
const HL_STRENGTH_PER_BRIGHT: f32 = 2.5;

// The Highlights slider as a tone curve: luminance in, luminance out. `h` is -1..+1 and
// `x_max` the brightest value the file can hold after exposure. Shapes and strengths are
// fitted to Lightroom's slider at -100 / -50 / +50 / +100. Acts above 0.1 scene-linear only.
fn highlight_tone(luma: f32, h: f32, x_max: f32) -> f32 {
    let pivot = 0.1;
    let over = max(luma - pivot, 0.0);
    let over_max = x_max - pivot;
    // Negative: pull the whole bright range down hard. The pivot stays put and the brightest
    // recordable value lands just under white, so a blown sun stays near-white with a long,
    // soft falloff around it.
    let ceiling = min(0.6, x_max);
    let keep = min(0.15, 0.5 * (ceiling - pivot) / over_max);
    let frac = clamp(over / over_max, 0.0, 1.0);
    // A little extra just above the pivot, so open sky is not pulled as far as the sun's glow.
    let near = 0.055 * (1.0 - exp(-over / 0.15));
    let rest = max(ceiling - pivot - over_max * keep - 0.055, 0.0);
    let pulled = pivot + over * keep + near + rest * pow(frac, 2.1);
    // Positive: up to about +1.4 stops, eased in just above the pivot.
    let pushed = luma + (over * over / (over + 0.05)) * (exp2(1.4 * max(h, 0.0)) - 1.0);
    // Between 0 and -100 the blend above alone would leave the brightest tones over white
    // until the slider is nearly at the end. Lightroom brings the sun's surroundings back
    // early, so add a shoulder that maps the brightest value in the file to `top`: equal to
    // that value at 0 (no effect), display white by -75, and the -100 ceiling at -100.
    let amount = clamp(-h, 0.0, 1.0);
    let blended = mix(luma, pulled, amount);
    let blended_max = mix(x_max, pivot + over_max * keep + 0.055 + rest, amount);
    let top = select(
        pow(max(x_max, 1.0), 1.0 - amount / 0.75),
        pow(ceiling, (amount - 0.75) / 0.25),
        amount > 0.75,
    );
    let knee = 0.35;
    let u = max(blended - knee, 0.0);
    let u_max = max(blended_max - knee, 1e-3);
    let v_max = clamp(top - knee, 1e-3, u_max);
    let recovered_tone = knee + u / (1.0 + u * (1.0 / v_max - 1.0 / u_max));
    let toned = select(select(blended, recovered_tone, blended > knee), pushed, h >= 0.0);
    return select(luma, toned, luma > pivot);
}

// How much of a colour boost to apply at a given scene luminance: full up to bright sky,
// then fading to nothing for tones that will render at or beyond white. Without this, the
// glow around a blown light source is pushed to one flat, fully saturated colour with a hard
// edge against the white core, instead of falling off smoothly into it.
fn bright_fade(luma: f32) -> f32 {
    return 1.0 - smoothstep(0.55, 2.0, luma);
}

// ---------------------------------------------------------------------------------------
// Lens correction
// ---------------------------------------------------------------------------------------

struct LensUv {
    r: vec2<f32>,
    g: vec2<f32>,
    b: vec2<f32>,
    // Radius of the source point in half-diagonals, for vignetting.
    rv: f32,
};

// Embedded correction table, linear between knots at (i + 0.5) / (n - 1) half-diagonals.
fn knot_value(r: f32, component: i32) -> f32 {
    let n = i32(params.lens_str.z);
    if (n < 2) {
        return 1.0;
    }
    let pos = clamp(r * f32(n - 1) - 0.5, 0.0, f32(n - 1));
    let i = min(i32(floor(pos)), n - 2);
    let t = pos - f32(i);
    let a = params.knots[i];
    let b = params.knots[i + 1];
    return mix(a[component], b[component], t);
}

// Where an output point of the sensor image is sampled from, per colour: profile and
// manual distortion, enlargement (constrain crop) and lateral chromatic aberration.
// Keep in step with `Gpu::warp` in lens.rs.
fn lens_coords(uv: vec2<f32>) -> LensUv {
    let size = vec2<f32>(textureDimensions(raw_texture, 0));
    let aspect = size.x / size.y;
    let shape = vec2<f32>(aspect, 1.0);
    let half_diag = length(shape);
    let c = (uv - vec2<f32>(0.5)) / params.lens_geo.w;
    let len = length(c * shape);
    let rh = len * 2.0 * params.lens_geo.x;
    let rd = len / half_diag * 2.0;
    let model = params.lens_dist.x;
    let k = params.lens_dist.yzw;
    var s = 1.0;
    if (model > 3.5) {
        s = knot_value(rd, 0);
    } else if (model > 2.5) {
        let r2 = rh * rh;
        s = 1.0 + k.x * r2 + k.y * r2 * r2;
    } else if (model > 1.5) {
        s = 1.0 - k.x + k.x * rh * rh;
    } else if (model > 0.5) {
        s = k.x * rh * rh * rh + k.y * rh * rh + k.z * rh + 1.0 - k.x - k.y - k.z;
    }
    s = 1.0 + params.lens_str.x * (s - 1.0);
    s *= 1.0 - params.lens_geo.z * rd * rd;
    let src = c * s;
    let rs = length(src * shape) * 2.0 * params.lens_geo.x;
    var sr = params.tca_r.x + params.tca_r.y * rs + params.tca_r.z * rs * rs;
    var sb = params.tca_b.x + params.tca_b.y * rs + params.tca_b.z * rs * rs;
    if (model > 3.5) {
        sr = knot_value(rd, 1);
        sb = knot_value(rd, 2);
    }
    sr *= 1.0 + params.tca_r.w;
    sb *= 1.0 + params.tca_b.w;
    var out: LensUv;
    out.g = src + vec2<f32>(0.5);
    out.r = src * sr + vec2<f32>(0.5);
    out.b = src * sb + vec2<f32>(0.5);
    out.rv = length(src * shape) / half_diag * 2.0;
    return out;
}

// Brightness correction for lens falloff (profile) and the manual vignette, in linear light.
fn lens_vignette(rv: f32) -> f32 {
    var gain = 1.0;
    if (params.vig.w > 1.5) {
        gain = 1.0 / max(knot_value(rv, 3), 1.0e-3);
    } else if (params.vig.w > 0.5) {
        let r2 = rv * params.lens_geo.y * rv * params.lens_geo.y;
        let falloff = 1.0 + params.vig.x * r2 + params.vig.y * r2 * r2 + params.vig.z * r2 * r2 * r2;
        gain = 1.0 / max(falloff, 0.05);
    }
    gain = pow(gain, params.lens_str.y);
    let manual = 1.0 + params.vig2.x * smoothstep(params.vig2.y, 1.42, rv * params.lens_geo.y);
    return gain * max(manual, 0.0);
}

// How close `hue` (0..6) is to `centre`, going round the circle.
fn hue_match(hue: f32, centre: f32) -> f32 {
    var d = abs(hue - centre);
    d = min(d, 6.0 - d);
    return 1.0 - smoothstep(0.22, 0.6, d);
}

// Axial (purple / green) fringe removal: on strong brightness edges, colours in the chosen
// purple or green range are pulled towards grey.
fn axial_defringe(color: vec3<f32>, uv: vec2<f32>, footprint: f32, texels: vec2<f32>) -> vec3<f32> {
    if (params.fringe.x <= 0.0 && params.fringe.z <= 0.0) {
        return color;
    }
    let lod = max(log2(max(footprint, 1.0)), 0.0);
    let step = vec2<f32>(1.5 * max(footprint, 1.0)) / texels;
    let e = sqrt(rough_luma(raw_at(uv + vec2<f32>(step.x, 0.0), lod)));
    let w = sqrt(rough_luma(raw_at(uv - vec2<f32>(step.x, 0.0), lod)));
    let n = sqrt(rough_luma(raw_at(uv - vec2<f32>(0.0, step.y), lod)));
    let s = sqrt(rough_luma(raw_at(uv + vec2<f32>(0.0, step.y), lod)));
    let edge = smoothstep(0.03, 0.12, max(abs(e - w), abs(n - s)));
    let hsv = rgb_to_hsv(color);
    let purple = params.fringe.x * hue_match(hsv.x, mix(4.3, 5.6, params.fringe.y));
    let green = params.fringe.z * hue_match(hsv.x, mix(1.4, 2.6, params.fringe.w));
    let amount = clamp(edge * smoothstep(0.06, 0.25, hsv.y) * max(purple, green), 0.0, 1.0);
    let grey = dot(color, params.luma.xyz);
    return mix(color, vec3<f32>(grey), amount);
}

// ---------------------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------------------

// The picture near `uv` at mip level `lod`, white-balanced and exposed like the main path.
fn work_at(uv: vec2<f32>, lod: f32) -> vec3<f32> {
    let cam = raw_at(uv, lod);
    return max(to_work(cam * params.wb.rgb / params.wb.w) * params.basic.x, vec3<f32>(0.0));
}

fn perceived(c: vec3<f32>) -> f32 {
    return sqrt(max(dot(c, params.luma.xyz), 0.0));
}

// Texture (fine detail), Clarity (local contrast) and Dehaze, from the picture's own mip
// levels. Each compares the pixel with a blurred version of its surroundings; the pixel is
// taken at the level of detail actually shown, so zoomed out it looks like a resized export.
// Brightness of the surroundings at a coarse scale, counting only what is similar to the
// pixel itself (bilateral weights), so a dark roof against the sky sees roof, not sky.
fn edge_aware_surroundings(uv: vec2<f32>, own: f32, lod: f32) -> f32 {
    let texels = vec2<f32>(textureDimensions(raw_texture, 0));
    let step = vec2<f32>(exp2(lod)) / texels;
    var total = 0.0;
    var weights = 0.0;
    for (var j = -2; j <= 2; j++) {
        for (var i = -2; i <= 2; i++) {
            let tap = perceived(work_at(uv + vec2<f32>(f32(i), f32(j)) * step, lod - 1.0));
            // Similarity in stops (perceived values are square roots of luminance).
            let stops = 2.0 * log2(max(tap, 1.0e-3) / max(own, 1.0e-3));
            let w = exp(-0.5 * stops * stops);
            total += w * tap;
            weights += w;
        }
    }
    return total / max(weights, 1.0e-4);
}

// Fitted to Lightroom exports of the reference file at -100, -50, +50 and +100.
const TEXTURE_FINE: f32 = 1.0;
const TEXTURE_LOD: f32 = 3.0;
const TEXTURE_STRENGTH: f32 = 0.68;
const TEXTURE_SOFTEN: f32 = 1.0;
const CLARITY_LOD: f32 = 6.0;
const CLARITY_STRENGTH: f32 = 1.4;
const CLARITY_LIFT: f32 = 0.6;
const CLARITY_LIFT_MEDIAN: f32 = 0.21;
const CLARITY_SHADE: vec2<f32> = vec2<f32>(0.35, 0.6);
const CLARITY_OWN: vec2<f32> = vec2<f32>(0.45, 0.7);
const CLARITY_LIFT_LOD: f32 = 7.0;
const DEHAZE_STRENGTH: f32 = 1.4;
const DEHAZE_POWER: f32 = 1.7;
const DEHAZE_DARKEN: f32 = 0.2;
const HAZE_DESATURATE: f32 = 0.3;
const HAZE_VEIL: f32 = 0.17;
const HAZE_POWER: f32 = 1.3;
const HAZE_PER_MEDIAN: f32 = 2.5;
const DEHAZE_PER_DARK: f32 = 1.8;
const DEHAZE_DARK_OFFSET: f32 = 0.45;

fn presence(color_in: vec3<f32>, uv: vec2<f32>, footprint: f32) -> vec3<f32> {
    let p = params.presence;
    if (abs(p.x) + abs(p.y) + abs(p.z) < 1.0e-4) {
        return color_in;
    }
    var color = color_in;
    let shown = clamp(log2(max(footprint, 1.0)), 0.0, 8.0);

    // Dehaze (atmospheric scattering, I = J t + A (1 - t)): airlight A from the brightest of
    // a coarse grid at level 8, a dark channel from level 3.5, then J = (I - A) / t + A.
    if (p.z != 0.0) {
        var brightest = 0.0;
        var brightest_colour = vec3<f32>(1.0);
        for (var j = 0; j < 4; j++) {
            for (var i = 0; i < 6; i++) {
                let sample = work_at(vec2<f32>((f32(i) + 0.5) / 6.0, (f32(j) + 0.5) / 4.0), 8.0);
                let l = dot(sample, params.luma.xyz);
                if (l > brightest) {
                    brightest = l;
                    brightest_colour = sample;
                }
            }
        }
        // A neutral airlight: taking its colour from the scene (a warm sun, say) would
        // shift every hue towards the complement. Capped so a blown sun does not set it.
        let airlight = vec3<f32>(clamp(brightest, 0.05, 1.0) * 0.95);
        if (p.z > 0.0) {
            // Haze removal, strongest where the dark channel is high (bright, washed-out
            // areas such as sky) and nearly nothing in darker, clear parts of the scene.
            // Measured against Lightroom: darkening the darker areas as the plain model
            // does is about three times too strong there.
            let base = work_at(uv, min(shown, 3.5));
            let low = work_at(uv, 3.5);
            let dark = clamp(min(low.r / airlight.r, min(low.g / airlight.g, low.b / airlight.b)), 0.0, 1.0);
            // Lightroom removes more haze when the photo's brightest areas are near-neutral (high
            // darkest channel on screen) and much less when they are strongly coloured.
            let pz = p.z * clamp(DEHAZE_PER_DARK * params.scene2.x - DEHAZE_DARK_OFFSET, 0.25, 1.3);
            let t = max(1.0 - DEHAZE_STRENGTH * pz * pow(dark, DEHAZE_POWER), 0.2);
            let clear = max((base - airlight) / t + airlight, vec3<f32>(0.0));
            // Each channel keeps at least a third of its value, so the sky beside the sun does
            // not lose its blue entirely.
            color *= clamp(clear / max(base, vec3<f32>(1.0e-4)), vec3<f32>(0.33), vec3<f32>(6.0));
            // Lightroom also deepens the whole picture a little.
            color *= exp2(-DEHAZE_DARKEN * pz);
        } else {
            // Adding haze: an even veil of the airlight over the whole picture, which lifts
            // the dark areas most, as Lightroom does.
            // Measured against Lightroom, the veil adds a roughly even amount of display
            // brightness to the darks and mid-tones (fading towards white), sized by how bright
            // the photo's brightest near-neutral areas are: mixed in a perceptual (square-root)
            // space towards a dark-channel airlight.
            // Stronger on bright, high-key photos.
            let k = HAZE_VEIL * pow(-p.z, HAZE_POWER) * (1.0 + HAZE_PER_MEDIAN * max(params.scene.z - 0.2, 0.0));
            // Airlight from the blank render's darkest channel (98th percentile on screen).
            let veil = pow(clamp(params.scene2.x, 0.05, 1.0), 1.1);
            // The veil takes the colour of the photo's brightest area (a blue-lit scene gets a
            // bluish veil, as in Lightroom).
            let air_tint = clamp(brightest_colour / max(dot(brightest_colour, params.luma.xyz), 1.0e-5), vec3<f32>(0.3), vec3<f32>(3.0));
            let mixed = sqrt(max(color, vec3<f32>(0.0))) * (1.0 - k) + veil * sqrt(air_tint) * k;
            color = mixed * mixed;
            // ...and washes the colour out, most visibly in the sky.
            let grey = dot(color, params.luma.xyz);
            color = mix(color, vec3<f32>(grey), HAZE_DESATURATE * pow(-p.z, 0.8));
        }
    }

    // Positive Clarity also lifts everything that sits in darker surroundings, as
    // Lightroom's does: on a backlit scene the foreground brightens a lot, the sky not at all.
    if (p.y > 0.0) {
        let surroundings = edge_aware_surroundings(uv, perceived(work_at(uv, 0.0)), CLARITY_LIFT_LOD);
        // Both the surroundings and the pixel itself must be dark: sky next to a dark roof is
        // not lifted, so no bright halo forms around silhouettes.
        let own = perceived(color);
        let shade = (1.0 - smoothstep(CLARITY_SHADE.x, CLARITY_SHADE.y, surroundings)) * (1.0 - smoothstep(CLARITY_OWN.x, CLARITY_OWN.y, own));
        // Only on low-key photos (dark median on screen); Lightroom leaves brighter ones alone.
        let low_key = clamp((CLARITY_LIFT_MEDIAN - params.scene.z) / 0.08, 0.0, 1.0);
        color *= exp2(CLARITY_LIFT * low_key * pow(p.y, 0.85) * shade);
    }
    let lp = perceived(color);
    var target_lp = lp;
    // Clarity: medium-scale contrast, kept to the mid-tones so silhouettes do not glow.
    if (p.y != 0.0) {
        // The base is edge-aware: across a strong edge (dark roof against the sky) the other side
        // barely counts, so the dark side is not pushed into a black band.
        let here = perceived(work_at(uv, min(shown, CLARITY_LOD)));
        let detail = here - edge_aware_surroundings(uv, here, CLARITY_LOD - 1.0);
        let mid_tones = smoothstep(0.05, 0.25, lp) * (1.0 - smoothstep(0.75, 0.98, lp));
        target_lp += CLARITY_STRENGTH * p.y * detail * mid_tones;
    }
    // Texture: fine detail, softened where it is already strong (edges, noise spikes).
    if (p.x != 0.0) {
        // The band between mip levels 1 and 3: finer than Clarity, coarser than Sharpening.
        let detail = perceived(work_at(uv, clamp(shown, TEXTURE_FINE, TEXTURE_LOD))) - perceived(work_at(uv, TEXTURE_LOD));
        // Lightroom takes much of the detail out already at -50, so the negative side ramps up faster.
        let amount = select(-TEXTURE_SOFTEN * pow(-p.x, 0.6), TEXTURE_STRENGTH * p.x, p.x > 0.0);
        target_lp += amount * detail / (1.0 + 6.0 * abs(detail));
    }
    let gain = clamp(max(target_lp, 0.0) / max(lp, 1.0e-3), 0.0, 3.0);
    return color * gain * gain;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Zoom / pan / rotation.
    // `frame` is a point of the straightened frame (the picture turned by the straighten
    // angle about its centre); `d` is where that falls on the displayed picture.
    let frame = (in.view - vec2<f32>(0.5)) * params.view.xy + params.view.zw;
    let q = frame - vec2<f32>(0.5);
    let st = params.straighten;
    let d = vec2<f32>(st.x * q.x - st.y * q.y / st.z, st.y * q.x * st.z + st.x * q.y) + vec2<f32>(0.5);
    // Where this point lies on the stored picture, then where the lens actually put it.
    let uv_out = vec2<f32>(dot(params.rot.xy, d), dot(params.rot.zw, d)) + params.rot_off.xy;
    let lens = lens_coords(uv_out);
    let uv = lens.g;
    // How many image pixels one screen pixel covers (needed before any branching).
    let texels = vec2<f32>(textureDimensions(raw_texture, 0));
    let footprint = max(length(dpdx(uv * texels)), length(dpdy(uv * texels)));
    let base_lod = max(log2(footprint), 0.0);
    var cam = vec3<f32>(
        raw_at(lens.r, base_lod).r,
        raw_at(uv, base_lod).g,
        raw_at(lens.b, base_lod).b,
    );
    if (d.x < 0.0 || d.y < 0.0 || d.x > 1.0 || d.y > 1.0) {
        discard;
    }
    // Lens correction can pull in points from outside the picture: leave those empty.
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
        discard;
    }
    if (frame.x < params.crop.x || frame.y < params.crop.y || frame.x > params.crop.z || frame.y > params.crop.w) {
        discard;
    }
    let LUMA = params.luma.xyz;

    // 0. Noise reduction, on the picture as stored (before white balance), at the level of
    // detail on screen.
    if (params.noise.x > 0.0 || params.noise.y > 0.0) {
        cam = reduce_noise(cam, uv, footprint, texels);
    }

    // 1. White balance with highlight reconstruction, then into the working space.
    var color = to_work(balance(cam));
    if (params.input.x > 1.5) {
        // Finished picture: plain white-balance gains (relative to how it was saved).
        color = to_work(cam * params.wb.rgb / params.wb.w);
    }
    // 1a. Lens vignetting (profile and manual) and axial fringes, in linear light.
    color *= lens_vignette(lens.rv);
    color = axial_defringe(color, uv, footprint, texels);

    // 1b. Sharpening: an unsharp mask on brightness only, worked out on a perceptual
    // (square-root) scale so dark and bright edges get the same treatment, and applied as a
    // gain so colour is untouched. It is defined at full-size image pixels; zoomed out, the
    // same mask is taken at the level of detail on screen and weakened by the amount a
    // downscale would wash it out, so "fit" looks like a resized export rather than crunchy.
    if (params.sharpen.x > 0.0) {
        let lod = max(log2(footprint), 0.0);
        let size = exp2(lod);
        let radius = params.sharpen.y;
        let reach = radius * size / texels;
        let w = vec3<f32>(0.25, 0.5, 0.25);
        let centre = sqrt(max(dot(cam, w), 0.0));
        var blur = 4.0 * centre;
        for (var j = -1; j <= 1; j++) {
            for (var i = -1; i <= 1; i++) {
                if (i == 0 && j == 0) {
                    continue;
                }
                let tap = raw_at(uv + vec2<f32>(f32(i), f32(j)) * reach, lod);
                let weight = select(1.0, 2.0, i == 0 || j == 0);
                blur += weight * sqrt(max(dot(tap, w), 0.0));
            }
        }
        var detail = centre - blur / 16.0;
        // Masking: leave smooth areas alone. Halo limit: big differences are compressed.
        let mask = smoothstep(params.sharpen.w * 0.5, params.sharpen.w * 1.5 + 1.0e-5, abs(detail));
        let limit = params.sharpen.z;
        detail = limit * tanh(detail / limit);
        let sigma = 0.8 * radius;
        let seen = 1.0 - exp(-19.74 * sigma * sigma * 0.25 / (size * size));
        let fade = seen / (1.0 - exp(-19.74 * sigma * sigma * 0.25));
        let sharp = max(centre + params.sharpen.x * fade * mask * detail, 0.0);
        let gain = clamp(sharp / max(centre, 1.0e-3), 0.0, 3.0);
        color *= gain * gain;
    }
    // Brightest value this file can hold: every channel saturated.
    let recovered_max = max(params.wb.r, max(params.wb.g, params.wb.b)) / params.wb.w;

    // Camera profile, part 1: the hue/saturation map corrects the camera's colour response.
    if (params.profile.z > 0.5) {
        let hsv = rgb_to_hsv(color);
        let entry = textureSampleLevel(hue_sat_table, table_sampler, table_coord(hsv, params.hue_sat_dims), 0.0).xyz;
        color = table_apply(hsv, entry, params.hue_sat_dims);
    }

    // 2. Exposure (includes the profile's baseline exposure).
    color *= params.basic.x;

    // 3. Tone sliders, all on linear light.
    // Blacks, fitted to Lightroom's slider at -100 / -50 / +50 / +100. Positive values are a
    // hue-preserving lift that is strongest in the deepest shadows (about 1.3 stops at +100)
    // and fades out towards the highlights. Negative values subtract a fixed amount from every
    // channel, which is what makes shadows crush to black and gain saturation on the way.
    var luma = dot(color, LUMA);
    var lp = 0.0;
    let bk = params.tone.w;
    if (bk >= 0.0) {
        color *= exp2(1.35 * bk / (1.0 + luma / 0.045));
    } else {
        color = max(color - vec3<f32>(0.04 * pow(-bk, 1.7)), vec3<f32>(0.0));
    }

    // Highlights, applied locally. The gain for a pixel comes mostly from the brightness of
    // its neighbourhood, not its own: a whole bright region is pulled down together, so the
    // differences inside it (cloud texture) survive instead of being flattened by the curve.
    // Neighbours much brighter or darker than the pixel (the other side of a silhouette) are
    // left out of that average, which keeps halos off hard edges. Hue is preserved.
    luma = dot(color, LUMA);
    let h = params.tone.y;
    let x_max = max(recovered_max * params.basic.x, 0.101);
    let base = luma * local_ratio(uv, cam);
    // On a darker photo Lightroom treats the top of the photo's own range as its highlights:
    // brightness is scaled up by how far the photo's 98th-percentile display level sits below
    // white before the curve, so the slider reaches as far down as it does on a bright photo.
    let hl_norm = pow(1.0 / clamp(params.scene.y, 0.2, 1.0), HIGHLIGHT_ADAPT);
    let gain_local = highlight_tone(base * hl_norm, h, x_max * hl_norm) / max(base * hl_norm, 1e-6);
    let gain_pixel = highlight_tone(luma * hl_norm, h, x_max * hl_norm) / max(luma * hl_norm, 1e-6);
    // Shadows, also local, fitted to Lightroom's slider at -100 / -50 / +50 / +100. Measured
    // there, the lift in stops grows steadily with how far the neighbourhood sits below a
    // reference level: about half a stop per stop at +100 (so the darkest areas come up four
    // stops or more), and about -0.7 stop per stop at -100. Bright areas are left alone.
    // Lightroom opens the shadows more the more bright area the photo has to balance them
    // against, and less when the whole photo is already bright: strength from the photo's
    // display levels, fitted per photo on nine Lightroom reference sets (1.0 on a backlit
    // sunset, about 0.2 on flat indoor shots).
    let shadow_strength = clamp(
        SHADOW_BASE + SHADOW_PER_BRIGHT * params.scene.x - SHADOW_PER_MEDIAN * max(params.scene.z - 0.15, 0.0),
        0.1,
        1.0,
    );
    let s = params.tone.z * shadow_strength;
    let shadow_ref = select(0.23, 0.47, s >= 0.0);
    let below = max(log2(shadow_ref / max(base, 2e-4)), 0.0);
    // Eased in over the first half stop so there is no visible edge at the reference level.
    let depth = below * below / (below + 0.5);
    let shadow_gain = exp2(select(0.72, 0.52, s >= 0.0) * s * depth);
    // How hard Lightroom pulls (or pushes) the highlights depends on the photo: about twice as
    // hard on low-key photos (dark median on screen), about half as hard on bright ones, and
    // less when much of the frame is bright. Applied as a power of the curve's gain.
    let hl_strength = clamp(HL_STRENGTH_BASE - HL_STRENGTH_PER_MEDIAN * params.scene.z, 0.4, 2.2)
        * clamp(1.0 - HL_STRENGTH_PER_BRIGHT * max(params.scene.x - 0.2, 0.0), 0.3, 1.0);
    let hl_gain = pow(max(mix(gain_pixel, gain_local, 0.9), 1e-4), hl_strength);
    color *= hl_gain * shadow_gain;

    // Whites, fitted to Lightroom's slider at -100 / -50 / +50 / +100. Positive values push
    // the bright end up by up to about 1.15 stops, growing quickly near the end of the slider
    // and leaving shadows alone. Negative values pull tones down in proportion to their
    // brightness. Hue is preserved.
    luma = dot(color, LUMA);
    lp = luma * luma;
    let wh = params.tone.x;
    // Positive Whites also moves the white point: the further the photo's brightest areas (98th
    // and 99.7th percentile display levels) sit below white, the more everything is brought up, by up to
    // about 2.6 stops at +100 on a dark photo. Steep near the end of the slider, as measured.
    let white_room = min(WHITES_ROOM_MAX, WHITES_ROOM_P98 * log2(1.0 / clamp(params.scene.y, 0.05, 1.0)) + WHITES_ROOM_P997 * log2(1.0 / clamp(params.scene.w, 0.05, 1.0)));
    let white_point = pow(max(wh, 0.0), WHITES_ROOM_POWER) * white_room * luma / (luma + 0.002);
    let whites_up = 1.15 * pow(max(wh, 0.0), 1.7) * lp / (lp + 0.0484) + white_point;
    let whites_down = -1.45 * pow(max(-wh, 0.0), 0.85) * min(luma, 0.9);
    color *= exp2(select(whites_down, whites_up, wh >= 0.0));

    // 3b. Presence: dehaze, clarity, texture.
    color = presence(color, uv, footprint);

    // 4. Contrast: power curve pivoting on linear mid-grey (0.18 stays 0.18).
    color = 0.18 * pow(color / 0.18, vec3<f32>(params.basic.y));

    // 5. Vibrance, fitted to Lightroom's slider at -100 / -50 / +50 / +100.
    {
        let grey = dot(color, LUMA);
        let hi = max(color.r, max(color.g, color.b));
        let lo = min(color.r, min(color.g, color.b));
        let sat_now = (hi - lo) / max(hi, 1e-6);
        let v = params.luma.w;
        if (v >= 0.0) {
            // Positive: boosts muted colours much more than saturated ones, and mid-tones
            // more than bright ones. Most of the effect is in the top half of the slider.
            let muted = (1.0 - sat_now) * (1.0 - sat_now);
            // The very darkest tones get less, or shadow noise would pick up colour.
            let tonal = (grey / (grey + 0.02)) / (1.0 + grey / 0.15);
            let boost = 4.8 * pow(v, 1.7) * muted * tonal * bright_fade(grey);
            color = max(mix(vec3<f32>(grey), color, 1.0 + boost), vec3<f32>(0.0));
        } else {
            // Negative: colours fade towards a grey that is darker than their own luminance
            // (about 25% darker for anything visibly coloured), and already-saturated
            // colours keep a little of their colour even at -100.
            let keep = 1.0 + v * (1.0 - 0.2 * sat_now);
            let fade_to = grey * (1.0 - 0.25 * smoothstep(0.03, 0.2, sat_now));
            color = mix(vec3<f32>(fade_to), color, keep);
        }
    }

    // Saturation: a plain scale on the distance from grey. -100 is fully grey; +100 is a
    // little under double, which is where Lightroom's slider lands.
    {
        let grey = dot(color, LUMA);
        let s = params.basic.z;
        // A boost fades out as the tone nears and passes display white (see bright_fade).
        let scale = 1.0 + s * select(1.0, 0.82 * bright_fade(grey), s >= 0.0);
        color = max(mix(vec3<f32>(grey), color, scale), vec3<f32>(0.0));
    }
    // Camera profile, part 2: the look table gives the profile its character.
    if (params.profile.w > 0.5) {
        let hsv = rgb_to_hsv(color);
        let entry = textureSampleLevel(look_table, table_sampler, table_coord(hsv, params.look_dims), 0.0).xyz;
        color = table_apply(hsv, entry, params.look_dims);
    }

    // Base tone curve.
    if (params.input.x > 1.5) {
        // Finished picture: it already carries its tone curve.
    } else if (params.profile.x > 1.5) {
        // Profile curve from the lookup table, applied the way Adobe does: the brightest and
        // darkest channels go through the curve and the middle one keeps its relative
        // position between them, which preserves hue.
        let c = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
        let hi = max(c.r, max(c.g, c.b));
        let lo = min(c.r, min(c.g, c.b));
        let hi2 = curve_lookup(hi);
        let lo2 = curve_lookup(lo);
        let scale = (hi2 - lo2) / max(hi - lo, 1e-6);
        color = select(vec3<f32>(lo2) + (c - vec3<f32>(lo)) * scale, vec3<f32>(hi2), hi - lo < 1e-6);
        // Optional extra contrast on top of the profile (the "Adobe Color" approximation):
        // an S-curve around 0.47 with matching slopes on both sides, per channel, so shadows
        // deepen and gain saturation while black and white stay put.
        let k = params.rot_off.z;
        if (k > 1.001) {
            let p = 0.47;
            // The curve above can land a hair outside 0..1 through rounding (a fully saturated
            // channel at exactly white, say). A power of a negative number is undefined and
            // shows up as a black pixel, so pin the range first.
            color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
            let low = p * pow(color / p, vec3<f32>(k));
            let high = vec3<f32>(1.0) - (1.0 - p) * pow((vec3<f32>(1.0) - color) / (1.0 - p), vec3<f32>(k));
            color = select(high, low, color < vec3<f32>(p));
        }
    } else if (params.profile.x > 0.5) {
        // Built-in fitted curve. Part luminance-based so hues hold, part per-channel so very
        // bright warm colours pale towards yellow-white.
        let base_grey = dot(color, LUMA);
        color = max(mix(vec3<f32>(base_grey), color, params.profile.y), vec3<f32>(0.0));
        let by_luma = color * (fitted_curve(vec3<f32>(base_grey)).x / max(base_grey, 1e-6));
        color = mix(by_luma, fitted_curve(color), 0.75);
    }

    // 5b. Color Mixer, or the black & white conversion, on the picture as displayed.
    if (params.mono_info.x > 0.5) {
        color = black_and_white(color);
    } else if (params.hsl_info.x > 0.5) {
        color = color_mixer(color);
    }

    // 5c. Tone curve (parametric regions and point curve) on a gamma 2.2 scale (fitted to
    // Lightroom's point-curve references), applied the way Adobe applies tone curves: the brightest and darkest
    // channels go through it and the middle one keeps its place between them, so hues hold
    // (a plain per-channel curve turns a sunset pink when the highlights come down).
    if (params.curve_info.x > 0.5) {
        let e = pow(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / CURVE_GAMMA));
        let hi = max(e.r, max(e.g, e.b));
        let lo = min(e.r, min(e.g, e.b));
        let hi2 = tone_curve(hi);
        let lo2 = tone_curve(lo);
        let scale = (hi2 - lo2) / max(hi - lo, 1e-6);
        let curved = select(vec3<f32>(lo2) + (e - vec3<f32>(lo)) * scale, vec3<f32>(hi2), hi - lo < 1e-6);
        color = pow(max(curved, vec3<f32>(0.0)), vec3<f32>(CURVE_GAMMA));
    }

    // Working space -> linear sRGB.
    color = vec3<f32>(
        dot(params.work_to_srgb_0.xyz, color),
        dot(params.work_to_srgb_1.xyz, color),
        dot(params.work_to_srgb_2.xyz, color),
    );

    // 6. Fit to the display. A bright saturated colour (orange sky near the sun) can have one
    //    channel above 1.0 while its luminance is still in range. Pale it towards its own
    //    luminance until it fits, rather than clipping that channel and shifting the hue.
    let out_luma = clamp(dot(color, REC709), 0.0, 1.0);
    let peak = max(color.r, max(color.g, color.b));
    let fit = select(1.0, (1.0 - out_luma) / max(peak - out_luma, 1e-6), peak > 1.0);
    let paled = vec3<f32>(out_luma) + (color - vec3<f32>(out_luma)) * fit;
    // Adobe profiles expect the plain per-channel clip instead: it is what turns a bright
    // orange sky yellow, then white, on the way into the sun.
    color = select(paled, color, params.profile.x > 1.5 || params.input.x > 1.5);

    // 6b. Clipping warning (viewport only, `input.y`): blown highlights in red when every
    // channel is at the top and amber when only some are; crushed blacks in blue.
    if (params.input.y > 0.5) {
        let top = max(color.r, max(color.g, color.b));
        let bottom = min(color.r, min(color.g, color.b));
        if (bottom >= 0.999) {
            color = vec3<f32>(0.90, 0.04, 0.04);
        } else if (top >= 0.999) {
            color = vec3<f32>(0.91, 0.34, 0.003);
        } else if (top <= 0.0005) {
            color = vec3<f32>(0.03, 0.14, 0.90);
        }
    }

    // 7. Display encoding (skipped when the surface format already encodes sRGB).
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let out_rgb = select(color, srgb_encode(color), params.basic.w > 0.5);
    return vec4<f32>(out_rgb, 1.0);
}
