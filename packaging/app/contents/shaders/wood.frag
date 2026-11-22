// Hi-Fi theme cabinet: a light oak veneer like a 70s/80s hi-fi rack, with
// long grain lines, slow ring figure and a soft lacquer highlight.
// Compiled to wood.frag.qsb by tools/build-shaders.sh.
#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    vec2 size;        // the cabinet in pixels
};

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

void main() {
    vec2 pixel = qt_TexCoord0 * size;
    // Vertical boards: the grain runs top to bottom.
    vec2 p = vec2(pixel.x / 11.0, pixel.y / 520.0);
    float warp = noise(p * vec2(0.25, 2.0)) * 1.4 + noise(p * vec2(1.0, 6.0)) * 0.35;
    float rings = 0.5 + 0.5 * sin((p.x + warp) * 6.2831853);
    float fibres = noise(vec2(pixel.x / 1.6, pixel.y / 60.0));

    vec3 light = vec3(0.78, 0.56, 0.34);
    vec3 dark = vec3(0.64, 0.43, 0.25);
    vec3 wood = mix(dark, light, smoothstep(0.05, 1.0, rings));
    wood *= 0.92 + 0.12 * fibres;
    // Lacquer: a soft vertical highlight.
    wood += 0.05 * exp(-pow(qt_TexCoord0.x - 0.35, 2.0) * 9.0);

    fragColor = vec4(clamp(wood, 0.0, 1.0), 1.0) * qt_Opacity;
}
