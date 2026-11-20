#!/usr/bin/env bash
# Spike: build the robroz client for the x86_64 emulator and pack an APK.
#   ROBROZ_ANDROID_HOST=http://10.0.2.2:PORT ./build.sh [debug|release]
set -euo pipefail
cd "$(dirname "$0")"
profile=${1:-debug}
rm -rf src && mkdir src && cp -r ../../client/src/* src/
mv src/main.rs src/lib.rs && cat android.rs >> src/lib.rs
rm -rf app/src/main/jniLibs
flags=(); [ "$profile" = release ] && flags=(--release)
cargo ndk --platform 26 -t x86_64 -o app/src/main/jniLibs build --lib "${flags[@]}"
task=assembleDebug; [ "$profile" = release ] && task=assembleRelease
gradle --quiet "$task"
ls app/build/outputs/apk/"$profile"/*.apk
