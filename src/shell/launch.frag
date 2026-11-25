precision highp float;
uniform vec2 size;
uniform float alpha;
uniform vec3 accent;
varying vec2 v_coords;

void main() {
    float width = min(size.x, size.y);
    float radius = min(15.0, width * 0.36);
    vec2 p = (v_coords - 0.5) * size;
    vec2 corner = abs(p) - size * 0.5 + radius + 0.5;
    float edge = length(max(corner, 0.0)) + min(max(corner.x, corner.y), 0.0) - radius;
    float fill = 1.0 - smoothstep(-0.55, 0.55, edge);
    float rim = fill - (1.0 - smoothstep(-0.55, 0.55, edge + 1.2));
    vec3 base = mix(vec3(0.07, 0.12, 0.22), vec3(0.18, 0.28, 0.44), 1.0 - v_coords.y);
    vec3 color = mix(base, accent, rim * 0.72);
    float glow = (1.0 - smoothstep(0.0, 12.0, length(p))) * (1.0 - smoothstep(35.0, 100.0, width));
    color = mix(color, accent, glow * 0.55);
    float opacity = alpha * fill;
    gl_FragColor = vec4(color * opacity, opacity);
}
