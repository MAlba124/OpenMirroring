#version 100

precision mediump float;

attribute vec2 position;
attribute vec2 inTextureCoords;
uniform mat4 transform;

varying vec2 textureCoords;

void main() {
    gl_Position = transform * vec4(position, 0.0, 1.0);
    textureCoords = inTextureCoords;
}