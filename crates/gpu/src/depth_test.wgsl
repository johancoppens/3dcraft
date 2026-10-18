// The depth test of the lines and the datum fills over the picture: the
// rule of `craft3d_ui_egui::viewport::surface_hides`, which the CPU
// viewport applies to its frame, on the scene pass's depth buffer. It is
// pasted into lines.wgsl and fills.wgsl at their marker and uses their
// `scene` and `lines` uniforms and `depth_texture`.

// The view distance of the surface under pixel `p`: the mean of its
// samples' depths, turned back into a distance. The depth is affine across
// a triangle on the screen (near / dist in perspective, far - dist scaled
// in orthographic) and the samples lie symmetric about the pixel's centre,
// so on a plane this is the distance at the centre exactly, where the CPU
// frame's sample is. Negative where a sample shows nothing (the cleared
// reversed depth, 0) or is not a number: such a pixel hides nothing.
fn surface_at(p: vec2<i32>) -> f32 {
    let samples = i32(lines.size.w);
    var sum = 0.0;
    for (var s = 0; s < samples; s = s + 1) {
        let z = {{LOAD}};
        // Negated, so that a depth that is not a number counts as nothing.
        if !(z > 0.0) {
            return -1.0;
        }
        sum = sum + z;
    }
    let z = sum / f32(samples);
    if scene.forward.w > 0.5 {
        // z = near / dist, as the scene pass writes it.
        return scene.depth.x / z;
    }
    // z = (far - dist) * scale.
    return scene.depth.x - z / scene.depth.y;
}

// How far the middle sample lies beyond the chord of its neighbours;
// nothing where it lies before it or a neighbour shows nothing.
fn beyond(p: f32, q: f32, r: f32) -> f32 {
    if p < 0.0 || q < 0.0 || r < 0.0 {
        return 0.0;
    }
    return max(2.0 * q - p - r, 0.0);
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
    var z: array<array<f32, 4>, 4>;
    for (var r = 0; r < 4; r = r + 1) {
        for (var k = 0; k < 4; k = k + 1) {
            let q = clamp(cell + vec2<i32>(k - 1, r - 1), vec2<i32>(0), size - vec2<i32>(1));
            z[r][k] = surface_at(q);
        }
    }
    let a = z[1][1];
    let b = z[1][2];
    let c = z[2][1];
    let d = z[2][2];
    if a < 0.0 || b < 0.0 || c < 0.0 || d < 0.0 {
        return false;
    }
    let surface = mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    var kink = 0.0;
    for (var n = 1; n < 3; n = n + 1) {
        // Row n of the cell at its two samples, and column n at its two.
        let row_first = beyond(z[n][0], z[n][1], z[n][2]);
        let row_second = beyond(z[n][1], z[n][2], z[n][3]);
        // Both by more than the slack: rounding makes no hollow.
        if row_first > lines.size.z && row_second > lines.size.z {
            kink = max(kink, max(row_first, row_second));
        }
        let column_first = beyond(z[0][n], z[1][n], z[2][n]);
        let column_second = beyond(z[1][n], z[2][n], z[3][n]);
        if column_first > lines.size.z && column_second > lines.size.z {
            kink = max(kink, max(column_first, column_second));
        }
    }
    let change = max(max(abs(b - a), abs(d - c)), max(abs(c - a), abs(d - b)));
    return surface + kink + change * lines.look.y + lines.size.z < depth;
}
