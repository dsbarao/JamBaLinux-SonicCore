// Vivo background "Ondas": four glowing sine lines along the bottom, drawn
// analytically per pixel (no geometry, so no jagged edges). Compiled to
// waves.frag.qsb by tools/build-shaders.sh.
#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    float phase;      // advances with time, faster with sound
    float pulse;      // 0..1 bass
    float strength;   // 0.22 / 0.38 / 0.56 intensity
    vec4 colorA;
    vec4 colorB;
};

void main() {
    vec2 uv = qt_TexCoord0;
    float pixel = fwidth(uv.y);
    vec3 color = vec3(0.0);
    float alpha = 0.0;
    for (int index = 0; index < 4; ++index) {
        float k = float(index);
        float base = 0.60 + k * 0.075;
        float amplitude = (0.02 + 0.07 * pulse) * (0.65 + 0.35 * sin(phase * 0.21 + k * 1.7));
        float y = base + amplitude * sin(uv.x * 6.2831853 * (1.1 + k * 0.33) + phase * (1.0 + k * 0.27) + k * 1.3);
        float distance = abs(uv.y - y);
        // A crisp 1.5 px core plus a soft halo.
        float core = 1.0 - smoothstep(0.0, 1.5 * pixel, distance);
        float halo = exp(-distance * 55.0) * 0.55;
        float weight = (core + halo) * strength * (1.4 + 0.8 * pulse);
        vec3 tone = mix(colorA.rgb, colorB.rgb, k / 3.0);
        color += tone * weight;
        alpha += weight;
    }
    alpha = clamp(alpha, 0.0, 1.0);
    // Premultiplied output.
    fragColor = vec4(min(color, vec3(alpha)), alpha) * qt_Opacity;
}
