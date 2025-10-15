#include <metal_stdlib>
using namespace metal;

struct VertexInput {
    packed_int2 pos;
    uint dim;
    uint uv;
    uint color;
    uint content_type_with_srgb;
    float depth;
};

struct VertexOutput {
    float4 position [[position]];
    float4 color;
    float2 uv;
    uint content_type [[flat]];
};

struct Params {
    uint2 screen_resolution;
};

float srgb_to_linear(float c) {
    if (c <= 0.04045) {
        return c / 12.92;
    } else {
        return pow((c + 0.055) / 1.055, 2.4);
    }
}

vertex VertexOutput vs_main(
    uint vertex_idx [[vertex_id]],
    uint instance_idx [[instance_id]],
    constant Params& params [[buffer(0)]],
    constant VertexInput* instances [[buffer(1)]],
    texture2d<float> color_atlas_texture [[texture(0)]],
    texture2d<float> mask_atlas_texture [[texture(1)]]
) {
    VertexInput in_vert = instances[instance_idx];
    int2 pos = in_vert.pos;
    uint width = in_vert.dim & 0xffffu;
    uint height = (in_vert.dim & 0xffff0000u) >> 16u;
    uint color = in_vert.color;
    uint2 uv = uint2(in_vert.uv & 0xffffu, (in_vert.uv & 0xffff0000u) >> 16u);

    uint2 corner_position = uint2(
        vertex_idx & 1u,
        (vertex_idx >> 1u) & 1u
    );
    uint2 corner_offset = uint2(width, height) * corner_position;

    uv = uv + corner_offset;
    pos = pos + int2(corner_offset);

    VertexOutput vert_output;
    vert_output.position = float4(
        2.0 * float2(pos) / float2(params.screen_resolution) - 1.0,
        in_vert.depth,
        1.0
    );
    vert_output.position.y *= -1.0;

    uint content_type = in_vert.content_type_with_srgb & 0xffffu;
    uint srgb = (in_vert.content_type_with_srgb & 0xffff0000u) >> 16u;

    if (srgb == 0u) {
        vert_output.color = float4(
            float((color & 0x00ff0000u) >> 16u) / 255.0,
            float((color & 0x0000ff00u) >> 8u) / 255.0,
            float(color & 0x000000ffu) / 255.0,
            float((color & 0xff000000u) >> 24u) / 255.0
        );
    } else if (srgb == 1u) {
        vert_output.color = float4(
            srgb_to_linear(float((color & 0x00ff0000u) >> 16u) / 255.0),
            srgb_to_linear(float((color & 0x0000ff00u) >> 8u) / 255.0),
            srgb_to_linear(float(color & 0x000000ffu) / 255.0),
            float((color & 0xff000000u) >> 24u) / 255.0
        );
    }

    uint2 dim = uint2(0u);
    if (content_type == 0u) {
        dim = uint2(color_atlas_texture.get_width(), color_atlas_texture.get_height());
    } else if (content_type == 1u) {
        dim = uint2(mask_atlas_texture.get_width(), mask_atlas_texture.get_height());
    }

    vert_output.content_type = content_type;
    vert_output.uv = float2(uv) / float2(dim);

    return vert_output;
}

fragment float4 fs_main(
    VertexOutput in_frag [[stage_in]],
    texture2d<float> color_atlas_texture [[texture(0)]],
    texture2d<float> mask_atlas_texture [[texture(1)]]
) {
    constexpr sampler atlas_sampler(coord::normalized, address::repeat, filter::linear);

    if (in_frag.content_type == 0u) {
        return color_atlas_texture.sample(atlas_sampler, in_frag.uv, level(0.0));
    } else if (in_frag.content_type == 1u) {
        float mask = mask_atlas_texture.sample(atlas_sampler, in_frag.uv, level(0.0)).x;
        return float4(in_frag.color.rgb, in_frag.color.a * mask);
    } else {
        return float4(0.0);
    }
}
