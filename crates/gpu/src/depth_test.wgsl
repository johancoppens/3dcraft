// The depth test of the lines and the datum fills over the picture: the
// rule of `craft3d_ui_egui::viewport::surface_hides`, which the CPU
// viewport applies to its frame, on the scene pass's depth buffer. It is
// pasted into lines.wgsl and fills.wgsl at their marker and uses their
// `scene` and `lines` uniforms and `distance_texture`.

// The view distance of the surface under pixel `p`, as distance.wgsl made
// it once per frame from the depth buffer: the mean of its
// samples' depths, turned back into a distance. The depth is affine across
// a triangle on the screen (near / dist in perspective, far - dist scaled
// in orthographic) and the standard sample positions lie symmetric about
// the pixel's centre (their mean is the centre), so on a plane this is
// the distance at the centre exactly, where the CPU frame's sample is.
// A surface may lie at a negative distance in orthographic (the eye plane
// inside the model's box), so "nothing" is a flag of its own, not a value:
// the sum is a surface only when every sample was drawn (the cleared
// reversed depth is 0, and a depth that is not a number counts as nothing).
struct Surface {
    distance: f32,
    drawn: bool,
}

fn surface_at(p: vec2<i32>) -> Surface {
    let distance = bitcast<f32>(textureLoad(distance_texture, p, 0).r);
    // Infinity (or anything that is not a finite number): nothing drawn.
    if !(abs(distance) < 3.0e38) {
        return Surface(0.0, false);
    }
    return Surface(distance, true);
}

// How far the middle sample lies beyond the chord of its neighbours;
// nothing where it lies before it or a neighbour shows nothing.
fn beyond(p: Surface, q: Surface, r: Surface) -> f32 {
    if !(p.drawn && q.drawn && r.drawn) {
        return 0.0;
    }
    return max(2.0 * q.distance - p.distance - r.distance, 0.0);
}

// Whether a surface of the picture lies in front of the point at pixel
// position `at` (x right, y down, in pixels) and view distance `depth` by
// more than the slack. The surface under the point is the bilinear
// interpolation of the four samples around it (exact on a plane); the
// allowance is the kink (along a row or column of the cell whose two
// samples both lie beyond the chord of their neighbours, the hollow of a
// crease between them, the larger of the two: what the interpolation
// misses there at most; nothing at a ridge or a step), the surface's
// change over the grain of a pixel (`lines.look.y`,
// `craft3d_render::SUBPIXEL_GRAIN`), and the slack.
fn surface_hides(at: vec2<f32>, depth: f32) -> bool {
    let size = vec2<i32>(i32(lines.size.x), i32(lines.size.y));
    // The samples lie at the pixel centres.
    let u = at - vec2<f32>(0.5, 0.5);
    let cell = vec2<i32>(floor(u));
    let f = u - vec2<f32>(cell);
    // The 4 by 4 samples, row r and column k at r * 4 + k: one array, as
    // GLSL ES 3.0 (WebGL2) has no arrays of arrays.
    var z: array<Surface, 16>;
    for (var r = 0; r < 4; r = r + 1) {
        for (var k = 0; k < 4; k = k + 1) {
            let q = clamp(cell + vec2<i32>(k - 1, r - 1), vec2<i32>(0), size - vec2<i32>(1));
            z[r * 4 + k] = surface_at(q);
        }
    }
    if !(z[5].drawn && z[6].drawn && z[9].drawn && z[10].drawn) {
        return false;
    }
    let a = z[5].distance;
    let b = z[6].distance;
    let c = z[9].distance;
    let d = z[10].distance;
    let surface = mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    var kink = 0.0;
    for (var n = 1; n < 3; n = n + 1) {
        // Row n of the cell at its two samples, and column n at its two.
        let row_first = beyond(z[n * 4], z[n * 4 + 1], z[n * 4 + 2]);
        let row_second = beyond(z[n * 4 + 1], z[n * 4 + 2], z[n * 4 + 3]);
        // Both by more than the slack: rounding makes no hollow.
        if row_first > lines.size.z && row_second > lines.size.z {
            kink = max(kink, max(row_first, row_second));
        }
        let column_first = beyond(z[n], z[4 + n], z[8 + n]);
        let column_second = beyond(z[4 + n], z[8 + n], z[12 + n]);
        if column_first > lines.size.z && column_second > lines.size.z {
            kink = max(kink, max(column_first, column_second));
        }
    }
    let change = max(max(abs(b - a), abs(d - c)), max(abs(c - a), abs(d - b)));
    return surface + kink + change * lines.look.y + lines.size.z < depth;
}
