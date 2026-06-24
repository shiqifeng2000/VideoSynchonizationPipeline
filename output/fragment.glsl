#version 330
in vec2 v_uv;
out vec4 color;

uniform sampler2D tex_y;
uniform sampler2D tex_uv;

void main() {
    float y = texture(tex_y, v_uv).r;
    vec2 uv = texture(tex_uv, v_uv).rg;

    float u = uv.x - 0.5;
    float v = uv.y - 0.5;

    vec3 rgb;
    rgb.r = y + 1.402 * v;
    rgb.g = y - 0.344 * u - 0.714 * v;
    rgb.b = y + 1.772 * u;

    color = vec4(rgb, 1.0);
}