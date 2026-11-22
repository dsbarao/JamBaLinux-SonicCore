// Hi-Fi theme faceplate: brushed aluminium in the style of an 80s hi-fi
// rack, drawn per pixel. A convex vertical shading, fine horizontal brush
// grain, a soft specular band, a machined bevel around the edge, and a pale
// sheen in the computer's lighting colors drifting across the metal (it
// brightens with the bass). Compiled to brushed.frag.qsb by
// tools/build-shaders.sh.
#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    float travel;     // advances with time, a little faster with sound
    float pulse;      // 0..1 bass
    float tint;       // 0 (plain silver) .. 1 strength of the light sheen
    vec2 size;        // the plate in pixels
    vec4 color0;      // the lights' palette, sampled at 0, 0.25, 0.5,
    vec4 color1;      // 0.75 and 1
    vec4 color2;
    vec4 color3;
    vec4 color4;
};

vec3 paletteAt(float position) {
    float scaled = fract(position) * 4.0;
    vec3 tone = mix(color0.rgb, color1.rgb, clamp(scaled, 0.0, 1.0));
    tone = mix(tone, color2.rgb, clamp(scaled - 1.0, 0.0, 1.0));
    tone = mix(tone, color3.rgb, clamp(scaled - 2.0, 0.0, 1.0));
    return mix(tone, color4.rgb, clamp(scaled - 3.0, 0.0, 1.0));
}

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Value noise stretched along x: long, thin brush strokes.
float grain(vec2 pixel) {
    vec2 cell = vec2(pixel.x / 90.0, pixel.y);
    vec2 i = floor(cell);
    vec2 f = fract(cell);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float along = mix(a, b, smoothstep(0.0, 1.0, f.x));
    // A second, finer layer so the strokes are not uniform.
    float fine = hash(vec2(floor(pixel.x / 7.0), pixel.y * 1.7));
    return along * 0.75 + fine * 0.25;
}

void main() {
    vec2 uv = qt_TexCoord0;
    vec2 pixel = uv * size;

    // Convex plate: lighter at the top, a little darker at the bottom.
    float shade = mix(0.80, 0.62, smoothstep(0.0, 1.0, uv.y));
    // Wide soft specular band across the upper third.
    shade += 0.09 * exp(-pow((uv.y - 0.20) * 4.5, 2.0));
    // Brush grain.
    shade += (grain(pixel) - 0.5) * 0.09;
    vec3 metal = vec3(shade) * vec3(0.985, 0.99, 1.0);

    // The lights' colors drifting across as a pale sheen: a slow diagonal
    // band plus a faint wash, so the plate keeps reading as silver.
    float diagonal = uv.x * 0.8 + uv.y * 0.35;
    vec3 light = paletteAt(diagonal * 0.6 - travel * 0.035);
    float band = 0.5 + 0.5 * sin((diagonal - travel * 0.09) * 6.2831853 * 0.7);
    float sheen = tint * (0.05 + 0.24 * pow(band, 3.0) + 0.14 * pulse);
    // Screen blend: the color lightens the metal instead of painting it.
    metal = 1.0 - (1.0 - metal) * (1.0 - light * sheen);

    // Machined bevel: a bright upper/left edge, a dark lower/right edge.
    float bevel = 3.0;
    float left = 1.0 - smoothstep(0.0, bevel, pixel.x);
    float top = 1.0 - smoothstep(0.0, bevel, pixel.y);
    float right = 1.0 - smoothstep(0.0, bevel, size.x - pixel.x);
    float bottom = 1.0 - smoothstep(0.0, bevel, size.y - pixel.y);
    metal += 0.22 * max(top, left);
    metal -= 0.30 * max(bottom, right);
    // A hairline groove just inside the edge.
    float inset = min(min(pixel.x, size.x - pixel.x), min(pixel.y, size.y - pixel.y));
    metal -= 0.06 * (1.0 - smoothstep(0.0, 1.0, abs(inset - 6.0)));

    fragColor = vec4(clamp(metal, 0.0, 1.0), 1.0) * qt_Opacity;
}
