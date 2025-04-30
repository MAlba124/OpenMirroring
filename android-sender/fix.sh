#!/usr/bin/env sh

set -x

objs=(
    libglib-2.0.so.0
    libgstwebrtc-1.0.so.0
    libgobject-2.0.so.0
    libgstnet-1.0.so.0
    libgstapp-1.0.so.0
    libgstbase-1.0.so.0
    libgstsdp-1.0.so.0
    libgstrtp-1.0.so.0
    libgstaudio-1.0.so.0
    libgstvideo-1.0.so.0
    libgio-2.0.so.0
    libgstpbutils-1.0.so.0
)

for obj in ${objs[@]}; do
    patchelf --remove-needed $obj ./app/src/main/jniLibs/x86_64/libomandroidsender.so
done

patchelf --replace-needed libgstreamer-1.0.so.0 libgstreamer_android.so ./app/src/main/jniLibs/x86_64/libomandroidsender.so

# patchelf --add-needed libgstreamer_android.so ./app/src/main/jniLibs/x86_64/libomandroidsender.so
