// A light beam in haze, drawn on the back faces of a bounding cone.
//
// The beam is a cone from a virtual apex behind the lens, so it leaves the
// fixture with the diameter of its LED field. Each pixel intersects its view
// ray with that cone, cuts the part behind the lens and behind scene geometry
// (from the depth prepass) and integrates the in-scattered light along it.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::{view, clustered_lights, globals};
import bevy_pbr::render::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT;
import bevy_pbr::render::clustered_forward as clustering;
import bevy_pbr::render::shadows::fetch_spot_shadow;
import bevy_pbr::prepass::utils::prepass_depth;
import bevy_pbr::render::view_transformations::{depth_ndc_to_view_z, position_world_to_view};

struct Beam {
    // xyz: virtual apex (world), w: axial distance of the lens from the apex.
    apex: vec4<f32>,
    // xyz: beam direction (world, unit), w: axial distance where the beam ends.
    axis: vec4<f32>,
    // xyz: gobo "up" (world, unit, perpendicular to axis), w: tan(outer half angle).
    up: vec4<f32>,
    // rgb: light color times intensity in candela, w: cos(inner half angle).
    color: vec4<f32>,
    // x: scattering coefficient (1/m), y: anisotropy g, z: cos(outer half angle),
    // w: most steps (negative: without jitter).
    haze: vec4<f32>,
    // rgb: haze tint, w: gobo: 0 none, 1 picture, 2 moving water.
    tint: vec4<f32>,
    // xyz: the beam's spot light (world), w: 1 to look for its shadow map.
    lens: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> beam: Beam;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var gobo_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var gobo_sampler: sampler;

const PI: f32 = 3.14159265;

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


// The clustered light behind this beam, if it casts shadows: its shadow map
// tells which parts of the beam objects block. -1 if there is none.
fn find_light(frag_xy: vec2<f32>, view_z: f32) -> i32 {
    let cluster = clustering::view_fragment_cluster_index(frag_xy, view_z, false);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    for (var i = ranges.first_spot_light_index_offset; i < ranges.first_rect_light_index_offset; i += 1u) {
        let id = clustering::get_clusterable_object_id(i);
        let light = &clustered_lights.data[id];
        if ((((*light).flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) && distance((*light).position_radius.xyz, beam.lens.xyz) < 0.01) {
            return i32(id);
        }
    }
    return -1;
}

// Interleaved gradient noise; varies per frame so TAA averages the step pattern away.
fn noise(p: vec2<f32>, frame: u32) -> f32 {
    let q = p + f32(frame % 64u) * vec2<f32>(5.588238, 5.588238);
    return fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let cam = view.world_position;
    let to_frag = in.world_position.xyz - cam;
    let t_frag = length(to_frag);
    let rd = to_frag / t_frag;

    let apex = beam.apex.xyz;
    let axis = beam.axis.xyz;
    let tan_h = beam.up.w;
    let cos2 = 1.0 / (1.0 + tan_h * tan_h);

    // Ray against the infinite cone around `axis`: (dot(p, axis))^2 = cos2 * |p|^2.
    let co = cam - apex;
    let dv = dot(rd, axis);
    let cv = dot(co, axis);
    let a = dv * dv - cos2;
    let b = 2.0 * (dv * cv - dot(rd, co) * cos2);
    let c = cv * cv - dot(co, co) * cos2;

    var t0 = 0.0;
    var t1 = t_frag;
    let inside = cv > 0.0 && c >= 0.0;
    if (!inside) {
        let disc = b * b - 4.0 * a * c;
        if (disc < 0.0 || abs(a) < 1e-6) {
            discard;
        }
        let s = sqrt(disc);
        let ra = (-b - s) / (2.0 * a);
        let rb = (-b + s) / (2.0 * a);
        // First hit on the forward half of the cone.
        var entry = 1e9;
        if (ra > 0.0 && cv + ra * dv > 0.0) { entry = ra; }
        if (rb > 0.0 && cv + rb * dv > 0.0) { entry = min(entry, rb); }
        if (entry > t1) {
            discard;
        }
        t0 = entry;
    }

    // Only between the lens plane and the end of the beam.
    if (abs(dv) > 1e-6) {
        let ts = (beam.apex.w - cv) / dv;
        let te = (beam.axis.w - cv) / dv;
        t0 = max(t0, min(ts, te));
        t1 = min(t1, max(ts, te));
    } else if (cv < beam.apex.w || cv > beam.axis.w) {
        discard;
    }

    // Stop at the first opaque surface.
    let frag_view_z = position_world_to_view(in.world_position.xyz).z;
    let scene_view_z = depth_ndc_to_view_z(prepass_depth(in.position, 0u));
    if (frag_view_z < -1e-4) {
        t1 = min(t1, t_frag * scene_view_z / frag_view_z);
    }
    if (t1 <= t0) {
        discard;
    }

    // About one step per 12 cm: short stretches through a beam need few.
    let max_steps = max(u32(abs(beam.haze.w)), 4u);
    let steps = clamp(u32(ceil((t1 - t0) / 0.12)), 4u, max_steps);
    let dt = (t1 - t0) / f32(steps);
    var jitter = 0.5;
    if (beam.haze.w > 0.0) {
        jitter = noise(in.position.xy, view.frame_count);
    }
    let g = beam.haze.y;
    let cos_outer = beam.haze.z;
    let cos_inner = beam.color.w;
    let up = beam.up.xyz;
    let right = cross(up, axis);

    // Objects in the way: sample the spot light's shadow map along the ray.
    var light = -1;
    if (beam.lens.w > 0.5) {
        let mid = cam + rd * (0.5 * (t0 + t1));
        light = find_light(in.position.xy, position_world_to_view(mid).z);
    }
    var near_z = 0.1;
    if (light >= 0) {
        near_z = clustered_lights.data[u32(light)].shadow_map_near_z;
    }

    var sum = 0.0;
    for (var i = 0u; i < steps; i += 1u) {
        let t = t0 + (f32(i) + jitter) * dt;
        let p = cam + rd * t;
        let l = p - apex;
        let r2 = max(dot(l, l), 1e-4);
        let dir = l * inverseSqrt(r2);
        let ca = dot(dir, axis);
        var spot = saturate((ca - cos_outer) / max(cos_inner - cos_outer, 1e-4));
        spot = spot * spot;
        if (beam.tint.w > 0.5 && spot > 0.0) {
            let z = max(dot(l, axis), 1e-4);
            let uv = vec2<f32>(dot(l, right), -dot(l, up)) / (z * tan_h) * 0.5 + vec2<f32>(0.5);
            if (beam.tint.w > 1.5) {
                spot *= water(uv, globals.time);
            } else {
                spot *= textureSampleLevel(gobo_texture, gobo_sampler, uv, 0.0).r;
            }
        }
        if (light >= 0 && spot > 0.0) {
            spot *= fetch_spot_shadow(u32(light), vec4<f32>(p, 1.0), vec3<f32>(0.0), near_z, in.position.xy);
        }
        // Henyey-Greenstein: light scattered from the beam towards the camera.
        let cos_t = dot(dir, -rd);
        let phase = (1.0 - g * g) / (4.0 * PI * pow(max(1.0 + g * g - 2.0 * g * cos_t, 1e-4), 1.5));
        sum += spot * phase / r2;
    }

    let radiance = beam.color.rgb * beam.tint.rgb * (beam.haze.x * sum * dt);
    // Alpha 0: Bevy blends `AlphaMode::Add` as premultiplied, so this adds to what's behind.
    return vec4<f32>(radiance * view.exposure, 0.0);
}
