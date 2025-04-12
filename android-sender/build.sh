#!/usr/bin/env sh

export ANDROID_HOME=/home/merb/Android/Sdk
export ANDROID_NDK_ROOT=/home/merb/Android/Sdk/ndk/29.0.13113456

cargo ndk --target x86_64-linux-android -o app/src/main/jniLibs build
