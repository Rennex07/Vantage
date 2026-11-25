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
uniform vec2 direction;
uniform vec2 texel;
varying vec2 v_coords;

vec3 pair(float distance) {
    return texture2D(tex, v_coords - distance * direction).rgb
         + texture2D(tex, v_coords + distance * direction).rgb;
}

void main() {
    vec3 color;
    if (direction.x == 0.0 && direction.y == 0.0) {
        // Prefilter the full-size scene before dropping to quarter resolution.
        color = 0.25 * (texture2D(tex, v_coords + texel).rgb
                      + texture2D(tex, v_coords - texel).rgb
                      + texture2D(tex, v_coords + vec2(texel.x, -texel.y)).rgb
                      + texture2D(tex, v_coords + vec2(-texel.x, texel.y)).rgb);
    } else {
        // A 17-tap Gaussian, with adjacent taps combined by linear filtering.
        color = texture2D(tex, v_coords).rgb * 0.103153;
        color += pair(1.476580) * 0.191011;
        color += pair(3.445530) * 0.140429;
        color += pair(5.414899) * 0.080715;
        color += pair(7.384912) * 0.036269;
    }
    gl_FragColor = vec4(color, 1.0) * alpha;
}
