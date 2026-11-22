// Vivo background "Partículas": soft glowing dots rising in three depth
// layers, generated per pixel from a hash (no textures, no geometry), so
// they stay perfectly smooth at any size. Compiled to particles.frag.qsb by
// tools/build-shaders.sh.
#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    float travel;     // how far the particles have risen; faster with sound
    float pulse;      // 0..1 bass
    float strength;   // intensity
    float aspect;     // width / height
    vec4 color0;      // the headset lights' palette, sampled at
    vec4 color1;      // 0, 0.25, 0.5, 0.75 and 1
    vec4 color2;
    vec4 color3;
    vec4 color4;
};

vec3 paletteAt(float position) {
    float scaled = clamp(position, 0.0, 1.0) * 4.0;
    vec3 tone = mix(color0.rgb, color1.rgb, clamp(scaled, 0.0, 1.0));
    tone = mix(tone, color2.rgb, clamp(scaled - 1.0, 0.0, 1.0));
    tone = mix(tone, color3.rgb, clamp(scaled - 2.0, 0.0, 1.0));
    return mix(tone, color4.rgb, clamp(scaled - 3.0, 0.0, 1.0));
}

float hash(vec2 point) {
    return fract(sin(dot(point, vec2(127.1, 311.7))) * 43758.5453);
}

void main() {
    vec2 uv = vec2(qt_TexCoord0.x * aspect, qt_TexCoord0.y);
    // Screen-space derivatives must be taken in uniform control flow:
    // inside a per-cell branch they are undefined at cell borders and drew
    // faint grid lines.
    float uvPixel = fwidth(uv.x);
    vec3 color = vec3(0.0);
    float alpha = 0.0;
    for (int layer = 0; layer < 3; ++layer) {
        float l = float(layer);
        float scale = 7.0 + l * 5.0;
        vec2 point = uv * scale;
        point.y += travel * (0.6 + l * 0.35);
        vec2 cell = floor(point);
        float seed = hash(cell + l * 17.0);
        // About half of the cells hold a dot (no branch: see uvPixel).
        float present = 1.0 - step(0.55, seed);
        // Each dot sits near its cell's centre and sways inside the cell,
        // so it stays round and never reaches the cell's edge.
        vec2 offset = (vec2(hash(cell + 3.1), hash(cell + 7.7)) - 0.5) * 0.24;
        offset.x += 0.08 * sin(travel * 1.3 + seed * 40.0);
        float distance = length(fract(point) - 0.5 - offset);
        float size = 0.05 + 0.05 * hash(cell + 1.9);
        float pixel = uvPixel * scale;
        float core = 1.0 - smoothstep(size - pixel, size + pixel, distance);
        // The glow fades out before the cell's edge: no seams between cells.
        float halo = exp(-distance * (12.0 - pulse * 4.0)) * 0.45
                   * (1.0 - smoothstep(0.18, 0.28, distance));
        float twinkle = 0.65 + 0.35 * sin(travel * 3.0 + seed * 40.0);
        float weight = present * (core + halo) * twinkle * strength * (1.2 + pulse) * (1.0 - l * 0.22);
        vec3 tone = paletteAt(qt_TexCoord0.x + (seed - 0.25));
        color += tone * weight;
        alpha += weight;
    }
    alpha = clamp(alpha, 0.0, 1.0);
    fragColor = vec4(min(color, vec3(alpha)), alpha) * qt_Opacity;
}
