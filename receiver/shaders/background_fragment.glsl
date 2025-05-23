// Sunset over the ocean. (https://www.shadertoy.com/view/wcjSzd)
// Minimalistic three color 2D shader
// inspired by this wonderful GIF: https://i.gifer.com/4Cb2.gif
//
// Features automatic anti aliasing by using smooth gradients
// removing the need for multi sampling.
//
// Revision 1:
//  - Add qnoise as approximation to value noise
//  - Inline remap() macro
//
// Copyright (c) srvstr 2025
// Licensed under MIT

// This is a placeholder

#version 100
precision mediump float;
varying vec2 frag_position;
uniform float effect_time;
uniform float win_width;
uniform float win_height;

/* Simple cosine based approximation of perlin noise.
 * Gives a more organic appearance.
 */
float cnoise(in vec2 uv) {
    // Rotation matrix with values corresponding to sin(1.7) and cos(1.7).
    const mat2 r = mat2(-0.1288, -0.9917, 0.9917, -0.1288);

    vec2 s0 = cos(uv);
    vec2 s1 = cos(uv * 2.5 * r);
    vec2 s2 = cos(uv * 4.0 * r * r);

    vec2 s = s0 * s1 * s2;

    return (s.x + s.y) * 0.25 + 0.5;
}

#define S(x) (smoothstep(0.0, 1.0, (x)))

void main() {
    vec2 uv = (frag_position - 0.5 * vec2(win_width, win_height)) / win_height;

    // Bias for smoothstep function to  simulate anti aliasing
    // with gradients.
    float dy = (smoothstep(0.0, -1.0, uv.y) * 40.0 + 1.5) / win_height;

    // Wave displacement factors.
    // XY: scale the UV coordinates for the noise.
    // Z: scales the noises strength.
    #define DISP_LENGTH 4
    vec3[DISP_LENGTH] disp;
    disp[0] = vec3(vec2( 0.5, 20.0), 8.0);
    disp[1] = vec3(vec2( 2.5, 60.0), 4.0);
    disp[2] = vec3(vec2( 5.0, 80.0), 2.0);
    disp[3] = vec3(vec2(10.0, 20.0), 2.0);

    float avg = 0.0;
    // Compute average of noise displacements
    for (int i = 0; i < DISP_LENGTH; i++) {
      avg += cnoise(uv * disp[i].xy +
                    effect_time * 0.5 /* 50% animation slowdown */) * disp[i].z - disp[i].z * 0.5;
    }
    avg /= float(DISP_LENGTH);

    // Displace vertically.
    vec2 st = vec2(uv.x, uv.y + clamp(avg * smoothstep(0.1, -1.0, uv.y), -0.1, 0.1));

    gl_FragColor = vec4(
        // Compose output gradients.
        mix(
            vec3(0.85, 0.55, 0),
            vec3(0.90, 0.40, 0),
            sqrt(abs(st.y * st.y * st.y)) * 28.0
        )
        /* Mask sun */
        * smoothstep(0.25 + dy, 0.25, length(st))
        /* Vingette + Background tint */
        + smoothstep(2.0, 0.5, length(uv)) * 0.1,
        1.0
    );
}
