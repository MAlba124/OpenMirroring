#version 100

precision mediump float;
attribute vec2 position;
varying vec2 frag_position;
uniform float win_width;
uniform float win_height;

void main() {
    frag_position = (position * 0.5 + 0.5) * vec2(win_width, win_height); // Map to pixel space
    gl_Position = vec4(position, 0.0, 1.0);
}
