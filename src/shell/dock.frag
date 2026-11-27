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
uniform vec2 wallpaper_origin;
uniform vec2 wallpaper_step;
uniform float panel_width;
uniform vec3 surface_color;
uniform vec3 glass_tint;
uniform float hovered;
uniform float pressed;
uniform vec2 launching;
uniform float solid;
uniform float reduce_motion;
varying vec2 v_coords;

float box(vec2 point, vec2 half_size, float radius) {
    vec2 corner = abs(point) - half_size + vec2(radius);
    return length(max(corner, 0.0)) + min(max(corner.x, corner.y), 0.0) - radius;
}

float aa(float distance_to_edge) {
    return 1.0 - smoothstep(-0.6, 0.6, distance_to_edge);
}

float segment(vec2 point, vec2 from, vec2 to) {
    vec2 ray = to - from;
    return length(point - from - ray * clamp(dot(point - from, ray) / dot(ray, ray), 0.0, 1.0));
}

vec3 blur_wallpaper(vec2 center) {
    vec2 d = 16.0 * wallpaper_step;
    vec2 farther = 29.0 * wallpaper_step;
    vec3 sum = texture2D(tex, center).rgb * 4.0;
    sum += texture2D(tex, center + vec2(d.x, 0.0)).rgb * 2.0;
    sum += texture2D(tex, center - vec2(d.x, 0.0)).rgb * 2.0;
    sum += texture2D(tex, center + vec2(0.0, d.y)).rgb * 2.0;
    sum += texture2D(tex, center - vec2(0.0, d.y)).rgb * 2.0;
    sum += texture2D(tex, center + d).rgb;
    sum += texture2D(tex, center - d).rgb;
    sum += texture2D(tex, center + vec2(d.x, -d.y)).rgb;
    sum += texture2D(tex, center + vec2(-d.x, d.y)).rgb;
    sum += texture2D(tex, center + vec2(farther.x, 0.0)).rgb;
    sum += texture2D(tex, center - vec2(farther.x, 0.0)).rgb;
    sum += texture2D(tex, center + vec2(0.0, farther.y)).rgb;
    sum += texture2D(tex, center - vec2(0.0, farther.y)).rgb;
    return sum / 20.0;
}

vec4 over(vec4 backdrop, vec3 color, float coverage) {
    return vec4(color * coverage, coverage) + backdrop * (1.0 - coverage);
}

vec4 app_icon(vec4 color, vec2 point, float index) {
    float hover = 1.0 - step(0.5, abs(hovered - index));
    float down = 1.0 - step(0.5, abs(pressed - index));
    float busy = index < 0.5 ? launching.x : index < 1.5 ? launching.y : 0.0;
    vec3 accent = index < 0.5 ? vec3(0.15, 0.31, 0.49) : vec3(0.53, 0.30, 0.11);
    vec2 p = point - vec2(42.0 + index * 62.0, 35.0 + (1.0 - reduce_motion) * (-2.0 * hover + 2.0 * down));
    p /= 1.0 - (1.0 - reduce_motion) * (0.10 * down + 0.12 * busy);
    float tile = box(p, vec2(24.0, 22.0), 16.0);
    color = over(color, vec3(1.0), aa(tile) * (0.30 * hover + 0.13 * busy));
    color = over(color, accent, aa(tile) * down * 0.20);

    if (index < 0.5) {
        float screen = box(p, vec2(14.5, 11.0), 4.5);
        float prompt = min(segment(p, vec2(-8.0, -4.0), vec2(-4.0, 0.0)),
                           segment(p, vec2(-4.0, 0.0), vec2(-8.0, 4.0)));
        float cursor = segment(p, vec2(0.5, 4.0), vec2(7.0, 4.0));
        color = over(color, vec3(0.09, 0.19, 0.31), aa(screen));
        float screen_rim = aa(screen - 0.55) - aa(screen + 0.65);
        color = over(color, vec3(0.80, 0.91, 1.0), screen_rim * 0.78);
        color = over(color, vec3(0.97, 0.99, 1.0), aa(min(prompt, cursor) - 1.05));
    } else if (index < 1.5) {
        float tab = box(p - vec2(-5.0, -4.0), vec2(9.5, 7.0), 3.0);
        float body = box(p - vec2(0.0, 2.0), vec2(14.5, 9.0), 3.5);
        color = over(color, accent, aa(min(tab, body)));
        color = over(color, vec3(0.80, 0.56, 0.29), aa(tab + 1.2));
        vec3 folder = mix(vec3(1.0, 0.84, 0.56), vec3(0.92, 0.65, 0.34),
                          clamp((p.y + 7.0) / 18.0, 0.0, 1.0));
        color = over(color, folder, aa(body + 1.2));
    } else if (42.0 + index * 62.0 > panel_width - 50.0) {
        float plate = length(p) - 13.0;
        float rim = aa(plate) - aa(plate + 1.0);
        color = over(color, vec3(0.09, 0.19, 0.31), aa(plate) * 0.70);
        color = over(color, vec3(0.84, 0.94, 1.0), rim * 0.75);
        float cross = min(segment(p, vec2(-5.0, 0.0), vec2(5.0, 0.0)),
                          segment(p, vec2(0.0, -5.0), vec2(0.0, 5.0)));
        color = over(color, vec3(1.0), aa(cross - 0.9));
    }
    float marker = box(point - vec2(42.0 + index * 62.0, 60.0), vec2(4.5, 1.2), 1.0);
    return over(color, accent, aa(marker) * max(hover * 0.56, busy));
}

void main() {
    vec2 point = (v_coords - wallpaper_origin) / wallpaper_step;
    vec2 local = point - vec2(panel_width * 0.5, 36.0);

    // The capsule normal keeps the thick frost on the top and around its shoulders.
    float spine = panel_width * 0.5 - 34.0;
    vec2 radial = local - vec2(clamp(local.x, -spine, spine), 0.0);
    float distance_from_axis = length(radial);
    vec2 normal = radial / max(distance_from_axis, 0.01);
    float panel = distance_from_axis - 30.0;
    float shape = aa(panel);
    float depth = max(-panel, 0.0);
    float upper = smoothstep(0.0, 0.94, -normal.y);
    vec3 ice = mix(vec3(0.90, 0.94, 1.0), glass_tint, 0.14);

    vec4 dock;
    if (solid > 0.5) {
        dock = vec4(surface_color, 1.0);
    } else {
        float refraction = exp(-depth / 5.0) * mix(3.5, 13.0, upper);
        vec2 sample_at = clamp(v_coords - normal * wallpaper_step * refraction, 0.0, 1.0);
        vec3 material = mix(blur_wallpaper(sample_at), glass_tint, 0.14);
        float opacity = 0.86;
        dock = vec4(material * opacity, opacity);
    }

    float frosted_edge = upper * (1.0 - smoothstep(0.9, 9.0, depth));
    dock = over(dock, ice, frosted_edge * 0.62);
    float outline = aa(panel + 0.25) - aa(panel + 1.45);
    dock = over(dock, vec3(1.0), outline * mix(0.64, 0.99, upper));
    dock = app_icon(dock, point, max(0.0, floor((point.x - 11.0) / 62.0)));
    dock *= shape;
    float shadow = (1.0 - smoothstep(0.0, 5.0, panel)) * 0.27;
    gl_FragColor = alpha * vec4(dock.rgb, dock.a + (1.0 - shape) * shadow);
}
