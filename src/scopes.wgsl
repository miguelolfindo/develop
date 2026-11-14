// Histogram and vectorscope, entirely on the GPU.
//
// `count` (compute) reads a small rendering of the finished picture and adds each pixel to
// bins with atomicAdd. `fs_scope` (fragment) draws either scope straight from those bins.
// Nothing is read back to the CPU.
//
// Bin layout: [0, 256) red, [256, 512) green, [512, 768) blue, [768, 1024) luminance,
// then a 128 x 128 Cb/Cr grid.

const HIST: u32 = 256u;
const GRID: u32 = 128u;
const GRID_START: u32 = 1024u;

// ---------------------------------------------------------------------------------------
// Counting
// ---------------------------------------------------------------------------------------

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> counts: array<atomic<u32>>;

@compute @workgroup_size(8, 8)
fn count(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let texel = textureLoad(source, vec2<i32>(id.xy), 0);
    if (texel.a < 0.5) {
        return; // outside the picture
    }
    let rgb = clamp(texel.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    atomicAdd(&counts[u32(rgb.r * 255.0 + 0.5)], 1u);
    atomicAdd(&counts[HIST + u32(rgb.g * 255.0 + 0.5)], 1u);
    atomicAdd(&counts[2u * HIST + u32(rgb.b * 255.0 + 0.5)], 1u);
    atomicAdd(&counts[3u * HIST + u32(luma * 255.0 + 0.5)], 1u);
    // Rec.709 colour difference, each -0.5..0.5.
    let cb = (rgb.b - luma) / 1.8556;
    let cr = (rgb.r - luma) / 1.5748;
    let gx = u32(clamp((cb + 0.5) * f32(GRID), 0.0, f32(GRID) - 1.0));
    let gy = u32(clamp((0.5 - cr) * f32(GRID), 0.0, f32(GRID) - 1.0));
    atomicAdd(&counts[GRID_START + gy * GRID + gx], 1u);
}

// ---------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------

struct Scope {
    // x = 0 histogram, 1 vectorscope. y = pixels counted. z = 1 if the target is not sRGB
    // (output display values as they are), 0 if it is (output linear). w = rect width / height.
    info: vec4<f32>,
};

@group(0) @binding(2) var<storage, read> bins: array<u32>;
@group(0) @binding(3) var<uniform> scope: Scope;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_scope(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

fn to_target(display: vec3<f32>) -> vec3<f32> {
    if (scope.info.z > 0.5) {
        return display;
    }
    return pow(display, vec3<f32>(2.2));
}

// The well both scopes are drawn in (#111317).
const WELL: vec3<f32> = vec3<f32>(0.067, 0.075, 0.090);

fn bar(bin: u32, scale: f32) -> vec3<f32> {
    // A bin holding 4x the average fills the height; the square root lifts the low end.
    return sqrt(min(vec3<f32>(f32(bins[bin]), f32(bins[HIST + bin]), f32(bins[2u * HIST + bin])) * scale, vec3<f32>(1.0)));
}

fn histogram(uv: vec2<f32>) -> vec3<f32> {
    let total = max(scope.info.y, 1.0);
    let bin = u32(clamp(uv.x, 0.0, 0.9999) * f32(HIST));
    let scale = f32(HIST) / (total * 4.0);
    let height = 1.0 - uv.y;
    let h = bar(bin, scale);
    var color = WELL;
    // Quarter lines.
    let quarter = abs(fract(uv.x * 4.0 + 0.5) - 0.5) / max(fwidth(uv.x * 4.0), 1.0e-5);
    color += vec3<f32>(0.035) * (1.0 - clamp(quarter, 0.0, 1.0)) * step(0.02, uv.x) * step(uv.x, 0.98);

    // Stack the three channels without blending: where all three reach, neutral white;
    // where two reach, their exact additive mix; where only the tallest reaches, its own colour.
    let lowest = min(h.r, min(h.g, h.b));
    let highest = max(h.r, max(h.g, h.b));
    let middle = h.r + h.g + h.b - lowest - highest;
    let reach = step(vec3<f32>(height), h); // 1 for each channel whose bar covers this pixel
    if (height <= lowest) {
        color = vec3<f32>(0.843, 0.863, 0.886); // #D7DCE2
    } else if (height <= middle) {
        color = reach * vec3<f32>(0.92, 0.88, 0.95); // yellow, cyan or magenta
    } else if (height <= highest) {
        color = reach * vec3<f32>(0.93, 0.78, 1.0); // red, green or blue
    }
    // A thin line along the top of each channel.
    let px = max(fwidth(height), 1.0e-5);
    let edges = vec3<f32>(1.0) - clamp(abs(vec3<f32>(height) - h) / (px * 1.2), vec3<f32>(0.0), vec3<f32>(1.0));
    color = mix(color, vec3<f32>(1.0, 0.36, 0.33), edges.r * 0.9);
    color = mix(color, vec3<f32>(0.33, 0.92, 0.45), edges.g * 0.9);
    color = mix(color, vec3<f32>(0.38, 0.62, 1.0), edges.b * 0.9);
    return color;
}

// Vectorscope scale: the outer ring (RING, as a fraction of the half height) stands for a
// chroma of RING_CHROMA. Keep in step with scopes.rs.
const RING: f32 = 0.92;
const RING_CHROMA: f32 = 0.45;

fn ring(radius: f32, at: f32, pixel: f32) -> f32 {
    return 1.0 - clamp(abs(radius - at) / pixel, 0.0, 1.0);
}

fn vectorscope(uv: vec2<f32>) -> vec3<f32> {
    let total = max(scope.info.y, 1.0);
    // Square scope centred in the rect; p is -1..1 with Cr up.
    let aspect = scope.info.w;
    let p = vec2<f32>((uv.x - 0.5) * 2.0 * max(aspect, 1.0), (0.5 - uv.y) * 2.0 * max(1.0 / aspect, 1.0));
    let radius = length(p);
    let pixel = max(fwidth(p.x), 1.0e-5);
    var color = WELL;

    // Graticule: rings at 25, 50, 75 and 100 %, and cross hairs.
    let rings = ring(radius, RING * 0.25, pixel) + ring(radius, RING * 0.5, pixel) + ring(radius, RING * 0.75, pixel);
    let cross = (1.0 - clamp(min(abs(p.x), abs(p.y)) / pixel, 0.0, 1.0)) * step(radius, RING);
    color += vec3<f32>(0.055) * rings + vec3<f32>(0.14) * ring(radius, RING, pixel) + vec3<f32>(0.05) * cross;

    // Targets for 75 % red, yellow, green, cyan, blue and magenta: small open boxes.
    var targets = array<vec2<f32>, 6>(
        vec2<f32>(-0.0859, 0.3750),
        vec2<f32>(-0.3750, 0.0344),
        vec2<f32>(-0.2891, -0.3406),
        vec2<f32>(0.0859, -0.3750),
        vec2<f32>(0.3750, -0.0344),
        vec2<f32>(0.2891, 0.3406),
    );
    for (var i = 0; i < 6; i++) {
        let d = abs(p - targets[i] * (RING / RING_CHROMA));
        let reach_d = max(d.x, d.y);
        color += vec3<f32>(0.30) * (1.0 - clamp(abs(reach_d - 0.045) / pixel, 0.0, 1.0));
    }

    // Skin-tone line: 123 degrees anticlockwise from +Cb, one pixel wide, anti-aliased.
    let skin = vec2<f32>(-0.5446, 0.8387);
    let along = dot(p, skin);
    let off = abs(p.x * skin.y - p.y * skin.x);
    let skin_line = (1.0 - clamp(off / pixel, 0.0, 1.0)) * step(0.0, along) * step(along, RING);
    color = mix(color, vec3<f32>(0.96, 0.62, 0.04), skin_line * 0.85);

    // The plot.
    let chroma = p * (RING_CHROMA / RING);
    if (radius <= RING + 0.04 && abs(chroma.x) < 0.5 && abs(chroma.y) < 0.5) {
        let gx = u32(clamp((chroma.x + 0.5) * f32(GRID), 0.0, f32(GRID) - 1.0));
        let gy = u32(clamp((0.5 - chroma.y) * f32(GRID), 0.0, f32(GRID) - 1.0));
        let hits = f32(bins[GRID_START + gy * GRID + gx]);
        let glow = 1.0 - exp(-hits * 2500.0 / total);
        // Tint each point with the colour it stands for.
        let hue = clamp(vec3<f32>(0.6 + 1.5748 * chroma.y, 0.6 - 0.1873 * chroma.x - 0.4681 * chroma.y, 0.6 + 1.8556 * chroma.x), vec3<f32>(0.0), vec3<f32>(1.0));
        color = mix(color, mix(hue, vec3<f32>(1.0), 0.35), glow);
    }
    return color;
}

@fragment
fn fs_scope(in: VsOut) -> @location(0) vec4<f32> {
    var color: vec3<f32>;
    if (scope.info.x < 0.5) {
        color = histogram(in.uv);
    } else {
        color = vectorscope(in.uv);
    }
    return vec4<f32>(to_target(color), 1.0);
}