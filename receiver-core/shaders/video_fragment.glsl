#version 100

precision mediump float;

varying vec2 textureCoords;
uniform sampler2D texture;

void main() {
     gl_FragColor = texture2D(texture, textureCoords);
}
