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
uniform float hovered;
uniform float pressed;
uniform vec2 launching;
uniform float workspace_position;
varying vec2 v_coords;

const vec3 ink = vec3(0.051, 0.086, 0.161);
const vec3 glass = vec3(0.106, 0.173, 0.302);
const vec3 blue = vec3(0.569, 0.749, 0.980);
const vec3 amber = vec3(0.925, 0.718, 0.486);
const vec3 paper = vec3(0.933, 0.957, 1.0);

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
    vec2 d = 12.0 * wallpaper_step;
    vec3 sum = texture2D(tex, center).rgb * 4.0;
    sum += texture2D(tex, center + vec2(d.x, 0.0)).rgb * 2.0;
    sum += texture2D(tex, center - vec2(d.x, 0.0)).rgb * 2.0;
    sum += texture2D(tex, center + vec2(0.0, d.y)).rgb * 2.0;
    sum += texture2D(tex, center - vec2(0.0, d.y)).rgb * 2.0;
    sum += texture2D(tex, center + d).rgb;
    sum += texture2D(tex, center - d).rgb;
    sum += texture2D(tex, center + vec2(d.x, -d.y)).rgb;
    sum += texture2D(tex, center + vec2(-d.x, d.y)).rgb;
    return sum / 16.0;
}

float digit(vec2 p, float slot) {
    float top = segment(p, vec2(-3.3, -5.0), vec2(2.9, -5.0));
    float middle = segment(p, vec2(-3.3, 0.0), vec2(2.9, 0.0));
    float bottom = segment(p, vec2(-3.3, 5.0), vec2(2.9, 5.0));
    float right = segment(p, vec2(2.9, -5.0), vec2(2.9, 0.0));
    if (slot < 0.5) {
        float stem = segment(p, vec2(0.4, -5.0), vec2(0.4, 5.0));
        float cap = segment(p, vec2(-2.8, -2.8), vec2(0.4, -5.0));
        return min(min(stem, cap), bottom);
    }
    float lower = slot < 1.5
        ? segment(p, vec2(-3.3, 0.0), vec2(-3.3, 5.0))
        : segment(p, vec2(2.9, 0.0), vec2(2.9, 5.0));
    return min(min(top, middle), min(bottom, min(right, lower)));
}

vec3 app_tile(vec3 color, vec2 point, float index) {
    float hover = 1.0 - step(0.5, abs(hovered - index));
    float down = 1.0 - step(0.5, abs(pressed - index));
    float busy = index < 0.5 ? launching.x : launching.y;
    vec3 accent = index < 0.5 ? blue : amber;
    vec2 p = point - vec2(42.0 + index * 62.0, 35.0 - 2.0 * hover + 2.0 * down);
    p /= 1.0 - 0.10 * down - 0.12 * busy;
    float tile = box(p, vec2(25.0, 24.0), 14.0);
    vec3 fill = mix(glass, accent, 0.13 + 0.16 * hover + 0.18 * busy);
    color = mix(color, mix(fill, ink, clamp((p.y + 24.0) / 72.0, 0.0, 0.70)), aa(tile));
    color = mix(color, accent, (aa(tile) - aa(tile + 1.0)) * (0.39 + 0.28 * hover));
    if (index < 0.5) {
        float screen = abs(box(p, vec2(14.0, 10.5), 4.0));
        float prompt = min(segment(p, vec2(-8.0, -4.0), vec2(-4.0, 0.0)),
                           segment(p, vec2(-4.0, 0.0), vec2(-8.0, 4.0)));
        float cursor = segment(p, vec2(0.5, 4.0), vec2(7.0, 4.0));
        color = mix(color, accent, aa(screen - 1.0));
        color = mix(color, paper, aa(min(prompt, cursor) - 1.1));
    } else {
        float tab = box(p - vec2(-5.0, -4.0), vec2(9.0, 7.0), 3.0);
        float body = box(p - vec2(0.0, 2.0), vec2(14.0, 9.0), 3.0);
        color = mix(color, amber * 0.72, aa(tab));
        color = mix(color, mix(amber, vec3(1.0, 0.88, 0.70), 0.30), aa(body));
        color = mix(color, vec3(0.97, 0.82, 0.61), aa(body + 1.0) - aa(body + 1.6));
    }
    float marker = box(point - vec2(42.0 + index * 62.0, 63.0), vec2(4.5, 1.2), 1.0);
    return mix(color, accent, aa(marker) * max(hover * 0.54, busy));
}

void main() {
    vec2 point = (v_coords - wallpaper_origin) / wallpaper_step;
    float panel = box(point - vec2(150.0, 36.0), vec2(147.0, 33.0), 26.0);
    float opacity = aa(panel);
    vec3 backdrop = mix(blur_wallpaper(v_coords), ink, 0.74);
    vec3 color = backdrop + 0.075 * (1.0 - smoothstep(4.0, 64.0, point.y));
    color = mix(color, blue, (opacity - aa(panel + 1.1)) * 0.36);

    if (point.x < 135.0) {
        color = app_tile(color, point, step(73.0, point.x));
    } else {
        float divider = box(point - vec2(138.0, 36.0), vec2(0.6, 15.0), 0.55);
        color = mix(color, blue, 0.26 * aa(divider));
        float tray = box(point - vec2(218.0, 36.0), vec2(68.0, 21.0), 18.0);
        color = mix(color, ink, 0.65 * aa(tray));
        float slot = clamp(floor((point.x - 155.0) / 42.0), 0.0, 2.0);
        vec2 number = point - vec2(176.0 + slot * 42.0, 36.0);
        float hover = 1.0 - step(0.5, abs(hovered - slot - 2.0));
        float down = 1.0 - step(0.5, abs(pressed - slot - 2.0));
        float chip = box(number, vec2(17.0, 15.0), 10.0);
        color = mix(color, blue, (0.14 * hover + 0.22 * down) * aa(chip));
        vec2 active = point - vec2(176.0 + workspace_position * 42.0, 36.0);
        float selected = box(active, vec2(17.0, 15.0), 10.0);
        color = mix(color, mix(glass, blue, 0.34), aa(selected) * 0.96);
        float dash = box(active - vec2(0.0, 10.0), vec2(3.0, 0.75), 0.7);
        color = mix(color, blue, aa(dash));
        color = mix(color, paper, aa(digit(number + vec2(0.0, 1.0), slot) - 0.85));
    }
    float shadow = (1.0 - smoothstep(0.0, 4.0, panel)) * 0.45;
    gl_FragColor = alpha * vec4(color * opacity, opacity + (1.0 - opacity) * shadow);
}
