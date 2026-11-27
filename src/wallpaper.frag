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
varying vec2 v_coords;

uniform vec3 tone_lift;
uniform vec3 tone_luma;

void main() {
    vec3 color = texture2D(tex, v_coords).rgb;
    float luminance = dot(color, vec3(0.2126, 0.7152, 0.0722));
    float highlight = smoothstep(0.04, 0.43, color.r - color.b);
    vec3 grade = tone_lift + color * mix(0.16, 0.61, highlight) + luminance * tone_luma * 1.48;
    gl_FragColor = vec4(clamp(grade, 0.0, 1.0) * alpha, alpha);
}
