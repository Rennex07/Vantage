#version 100
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

precision highp float;
uniform float alpha;
uniform vec2 panel_origin;
uniform vec2 output_size;
uniform vec2 panel_size;
uniform float corner_radius;
uniform vec3 surface_color;
uniform float daylight;
uniform float full_quality;
varying vec2 v_coords;

void main() {
    vec2 point = (v_coords - panel_origin) * output_size;
    float radius = min(corner_radius, 0.5 * min(panel_size.x, panel_size.y));
    vec2 corner = abs(point - panel_size * 0.5) - panel_size * 0.5 + radius;
    float edge = length(max(corner, 0.0)) + min(max(corner.x, corner.y), 0.0) - radius;
    float coverage = 1.0 - smoothstep(-0.7, 0.5, edge);
    float rim = coverage - (1.0 - smoothstep(-0.7, 0.5, edge + 1.2));

    // The frosted center diffuses the scene. A narrow bevel also bends the
    // backdrop toward the panel center, creating a clear rim around the frost.
    vec2 outward = (point - panel_size * 0.5) / max(panel_size * 0.5, vec2(1.0));
    float bevel = exp(min(0.0, edge) / 8.0);
    vec2 refracted = clamp(v_coords - outward * bevel * 15.0 / output_size, 0.0, 1.0);
    vec3 blurred = texture2D(tex, refracted).rgb;
    vec3 reflected = blurred;
    if (full_quality > 0.5) {
        reflected = texture2D(tex, clamp(refracted - outward * 7.0 / output_size, 0.0, 1.0)).rgb;
    }
    float luminance = dot(blurred, vec3(0.2126, 0.7152, 0.0722));
    float height = clamp(point.y / panel_size.y, 0.0, 1.0);
    vec3 material = mix(blurred, vec3(luminance), 0.50);
    float transmission = mix(0.53 / (1.0 + 12.0 * luminance * luminance), 0.35, daylight);
    vec3 color = surface_color * mix(1.06, 0.94, height);
    color = mix(color, material, transmission);
    color += (reflected - color) * bevel * 0.08;
    float highlight = clamp(0.52 - 0.35 * outward.x - 0.38 * outward.y, 0.13, 1.0);
    float crown = 1.0 - smoothstep(0.0, 10.0, max(-edge, 0.0));
    float upper = smoothstep(0.35, 0.90, -outward.y);
    vec3 reflection = mix(vec3(0.92, 0.96, 1.0), vec3(1.0, 0.98, 0.94), daylight);
    color = mix(color, reflection, crown * (0.16 * highlight + 0.58 * upper));
    if (full_quality > 0.5) {
        float grain = fract(52.9829189 * fract(dot(floor(point), vec2(0.06711056, 0.00583715))));
        color += (grain - 0.5) * 0.024;
    }
    color = mix(color, reflection, rim * (0.46 + 0.49 * upper));
    float opacity = alpha * coverage;
    gl_FragColor = vec4(color * opacity, opacity);
}
