// An LED or lens: dazzling when seen from inside the beam, a saturated glow
// from the side (real optics send almost all light into the beam).

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

struct Lens {
    // rgb: luminance seen from the side (nits), w: gain when looking into the beam.
    color: vec4<f32>,
    // x: cos(outer half angle), y: cos(inner half angle).
    cone: vec4<f32>,
    // xyz: beam direction (world, unit).
    axis: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> lens: Lens;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = lens.axis.xyz;
    let to_cam = normalize(view.world_position - in.world_position.xyz);
    let c = dot(n, to_cam);
    var inside = saturate((c - lens.cone.x) / max(lens.cone.y - lens.cone.x, 1e-4));
    inside = inside * inside;
    let gain = 1.0 + (lens.color.w - 1.0) * inside;
    // Seen from behind, the lens is dark.
    let front = select(0.0, 1.0, c > 0.0);
    return vec4<f32>(lens.color.rgb * gain * front * view.exposure, 1.0);
}
