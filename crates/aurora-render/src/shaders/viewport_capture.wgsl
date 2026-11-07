// A small read-only viewport sample. Output bytes are opaque, top-down sRGB.
struct Params { rect: vec4<f32>, source_size: vec2<f32>, encode_srgb: u32, _pad: u32 };
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;
struct Out { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) vertex: u32) -> Out {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    var result: Out;
    result.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    result.uv = uv;
    return result;
}
fn srgb(color: vec3<f32>) -> vec3<f32> {
    let value = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(value * 12.92, 1.055 * pow(value, vec3<f32>(1.0 / 2.4)) - 0.055, value > vec3<f32>(0.0031308));
}
@fragment fn fs_main(input: Out) -> @location(0) vec4<f32> {
    let half_texel = 0.5 / params.source_size;
    let uv = clamp(params.rect.xy + input.uv * params.rect.zw,
        params.rect.xy + half_texel, params.rect.xy + params.rect.zw - half_texel);
    var color = textureSampleLevel(source, source_sampler, uv, 0.0).rgb;
    // An sRGB texture view already decoded its sample. Unorm presentation
    // pixels already contain display encoding and must not be encoded twice.
    if params.encode_srgb != 0u { color = srgb(color); }
    return vec4<f32>(color, 1.0);
}
