#!/usr/bin/env sh

set -x

patchelf --remove-needed libglib-2.0.so.0 ./app/src/main/jniLibs/x86_64/libomandroidsender.so
patchelf --replace-needed libgstreamer-1.0.so.0 libgstreamer_android.so ./app/src/main/jniLibs/x86_64/libomandroidsender.so
