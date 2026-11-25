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
varying vec2 v_coords;

void main() {
    vec2 point = (v_coords - panel_origin) * output_size;
    float radius = min(11.0, 0.5 * min(panel_size.x, panel_size.y));
    vec2 corner = abs(point - panel_size * 0.5) - panel_size * 0.5 + radius;
    float edge = length(max(corner, 0.0)) + min(max(corner.x, corner.y), 0.0) - radius;
    float coverage = 1.0 - smoothstep(-0.7, 0.5, edge);
    float rim = coverage - (1.0 - smoothstep(-0.7, 0.5, edge + 1.2));

    float height = clamp(point.y / panel_size.y, 0.0, 1.0);
    vec3 tone = mix(vec3(0.13, 0.22, 0.37), vec3(0.050, 0.091, 0.168), height);
    vec3 color = mix(texture2D(tex, v_coords).rgb, tone, 0.52);
    float grain = fract(52.9829189 * fract(dot(floor(point), vec2(0.06711056, 0.00583715))));
    color += (grain - 0.5) * 0.024;
    color = mix(color, vec3(0.62, 0.78, 1.0), rim * 0.42);
    float opacity = alpha * coverage;
    gl_FragColor = vec4(color * opacity, opacity);
}
