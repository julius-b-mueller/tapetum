// Every stage surface (floor, set, objects, gauze): Bevy's standard lighting,
// minus the light a gobo blocks. For each gobo lamp the light it adds here is
// computed exactly as Bevy does (diffuse, specular, shadow, light through
// translucent foil) and the share the gobo blocks is taken away again. Light
// of other lamps and of the house light stays untouched.

import bevy_pbr::render::pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing, calculate_diffuse_color, calculate_F0_dielectric};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import bevy_pbr::render::pbr_lighting as lighting;
import bevy_pbr::render::pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT};
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::mesh_view_bindings::{view, globals, clustered_lights};
import bevy_pbr::render::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT;
import bevy_pbr::render::clustered_forward as clustering;
import bevy_pbr::render::shadows::fetch_spot_shadow;
import bevy_pbr::render::view_transformations::position_world_to_view;
import bevy_pbr::render::mesh_types::MESH_FLAGS_TRANSMITTED_SHADOW_RECEIVER_BIT;

// [0].x: number of gobos; then 4 entries per gobo:
//   lens: xyz spot light position (world), w mask layer
//   axis: xyz beam direction (unit), w range (m)
//   up:   xyz gobo "up" (unit), w tan(outer half angle)
//   cone: x cos(outer half angle), y light left in blocked parts, z 1 = water
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage, read> gobos: array<vec4<f32>, 129>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var gobo_masks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var gobo_sampler: sampler;

// Moving water caustics in gobo coordinates (0..1), cut to the round aperture.
// After the well-known iterated-sine caustic by joltz0r (shadertoy MdlXz8).
fn water(uv: vec2<f32>, time: f32) -> f32 {
    if (length((uv - vec2<f32>(0.5)) * 2.0) > 0.94) {
        return 0.0;
    }
    let t = time * 0.5 + 23.0;
    // Far from the origin, so no term divides by zero.
    let p = uv * 6.2831853 * 1.5 - vec2<f32>(250.0);
    var i = p;
    var c = 1.0;
    let inten = 0.005;
    for (var n = 0; n < 5; n += 1) {
        let tn = t * (1.0 - 3.5 / f32(n + 1));
        i = p + vec2<f32>(cos(tn - i.x) + sin(tn + i.y), sin(tn - i.y) + cos(tn + i.x));
        c += 1.0 / length(vec2<f32>(p.x / (sin(i.x + tn) / inten), p.y / (cos(i.y + tn) / inten)));
    }
    c /= 5.0;
    c = 1.17 - pow(c, 1.4);
    return clamp(0.15 + pow(abs(c), 8.0) * 1.2, 0.0, 1.0);
}

// The clustered spot light at `lens`, or -1.
fn find_light(frag_xy: vec2<f32>, view_z: f32, lens: vec3<f32>) -> i32 {
    let cluster = clustering::view_fragment_cluster_index(frag_xy, view_z, false);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    for (var i = ranges.first_spot_light_index_offset; i < ranges.first_rect_light_index_offset; i += 1u) {
        let id = clustering::get_clusterable_object_id(i);
        if (distance(clustered_lights.data[id].position_radius.xyz, lens) < 0.01) {
            return i32(id);
        }
    }
    return -1;
}

fn shadow(light: u32, p: vec4<f32>, normal: vec3<f32>, frag_xy: vec2<f32>) -> f32 {
    if ((clustered_lights.data[light].flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
        return 1.0;
    }
    return fetch_spot_shadow(light, p, normal, clustered_lights.data[light].shadow_map_near_z, frag_xy);
}

// The light the gobos hold back at this point (before exposure).
fn blocked_light(in: PbrInput) -> vec3<f32> {
    let count = min(u32(gobos[0].x), 32u);
    if (count == 0u) {
        return vec3<f32>(0.0);
    }
    let p = in.world_position.xyz;
    let m = in.material;

    // The same inputs `apply_pbr_lighting` builds (base layer only).
    let NdotV = max(dot(in.N, in.V), 0.0001);
    var input: lighting::LightingInput;
    input.layers[0].NdotV = NdotV;
    input.layers[0].N = in.N;
    input.layers[0].R = reflect(-in.V, in.N);
    input.layers[0].perceptual_roughness = m.perceptual_roughness;
    input.layers[0].roughness = lighting::perceptualRoughnessToRoughness(m.perceptual_roughness);
    input.P = p;
    input.V = in.V;
    input.diffuse_color = calculate_diffuse_color(m.base_color.rgb, m.metallic, m.specular_transmission, m.diffuse_transmission);
    input.metallic = m.metallic;
    input.F0_dielectric = calculate_F0_dielectric(m.reflectance);
    input.F0_metallic = m.base_color.rgb;
    input.F_ab = lighting::F_AB(m.perceptual_roughness, NdotV);

    // Light shining through translucent foil from behind (shadowed like Bevy
    // does it: only for meshes marked as transmitted shadow receivers).
    let translucent = m.diffuse_transmission > 0.0;
    let back_shadows = (in.flags & MESH_FLAGS_TRANSMITTED_SHADOW_RECEIVER_BIT) != 0u;
    let back = in.world_position - vec4<f32>(in.world_normal, 0.0) * m.thickness;
    var through: lighting::LightingInput;
    through.layers[0].NdotV = 1.0;
    through.layers[0].N = -in.N;
    through.layers[0].perceptual_roughness = 1.0;
    through.layers[0].roughness = 1.0;
    through.P = back.xyz;
    through.V = -in.V;
    through.diffuse_color = m.base_color.rgb * (1.0 - m.metallic) * (1.0 - m.specular_transmission) * m.diffuse_transmission;
    through.F_ab = vec2<f32>(0.1);

    var total = vec3<f32>(0.0);
    for (var k = 0u; k < count; k += 1u) {
        let lens = gobos[1u + 4u * k];
        let axis = gobos[2u + 4u * k];
        let up = gobos[3u + 4u * k];
        let cone = gobos[4u + 4u * k];
        let l = p - lens.xyz;
        let z = dot(l, axis.xyz);
        if (z <= 0.01 || z > axis.w || dot(normalize(l), axis.xyz) < cone.x) {
            continue;
        }
        // The gobo's image plane: x to the right, y up as seen along the beam.
        let right = cross(up.xyz, axis.xyz);
        let uv = vec2<f32>(dot(l, right), -dot(l, up.xyz)) / (z * up.w) * 0.5 + vec2<f32>(0.5);
        var mask = textureSampleLevel(gobo_masks, gobo_sampler, uv, i32(lens.w), 0.0).r;
        if (cone.z > 0.5) {
            mask = water(uv, globals.time);
        }
        let blocked = (1.0 - cone.y) * (1.0 - mask);
        if (blocked <= 0.001) {
            continue;
        }
        let light = find_light(in.frag_coord.xy, position_world_to_view(p).z, lens.xyz);
        if (light < 0) {
            continue;
        }
        let id = u32(light);
        var lit = lighting::spot_light(id, &input, true) * shadow(id, in.world_position, in.world_normal, in.frag_coord.xy);
        if (translucent) {
            var back_shadow = 1.0;
            if (back_shadows) {
                back_shadow = shadow(id, back, -in.world_normal, in.frag_coord.xy);
            }
            lit += lighting::spot_light(id, &through, true) * back_shadow;
        }
        total += lit * blocked;
    }
    return total;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material.flags, pbr_input.material.alpha_cutoff, pbr_input.material.base_color);
    var out: FragmentOutput;
    if ((pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u) {
        out.color = apply_pbr_lighting(pbr_input);
        out.color = vec4<f32>(max(out.color.rgb - view.exposure * blocked_light(pbr_input), vec3<f32>(0.0)), out.color.a);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
