#!/usr/bin/env bash
# Called by Xcode (see project.yml) with the cargo binary name. Builds
# `robroz-mobile` for the iOS device (or simulator) target and puts the
# executable where Xcode expects it. Does not sign: Xcode runs with
# CODE_SIGNING_ALLOWED=NO in CI and the owner signs the .ipa himself.
set -euxo pipefail

# Xcode runs scripts with a bare PATH; add cargo (rust-lang/rust#80817).
export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH:$HOME/.cargo/bin"

if [[ "${CONFIGURATION:-Release}" != "Debug" ]]; then
  PROFILE=release
  MAYBE_RELEASE=--release
else
  PROFILE=debug
  MAYBE_RELEASE=
fi

# Keep cargo's cache inside Xcode's derived files.
export CARGO_TARGET_DIR="$DERIVED_FILE_DIR/cargo"

if [[ "${LLVM_TARGET_TRIPLE_SUFFIX-}" == "-simulator" ]]; then
  TARGET=aarch64-apple-ios-sim
else
  TARGET=aarch64-apple-ios
fi

# The workspace root is two levels up from mobile/ios.
cd "$SRCROOT/../.."
cargo build $MAYBE_RELEASE --target "$TARGET" -p robroz-mobile --bin "$1"

mkdir -p "$(dirname "$TARGET_BUILD_DIR/$EXECUTABLE_PATH")"
cp "$CARGO_TARGET_DIR/$TARGET/$PROFILE/$1" "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"
