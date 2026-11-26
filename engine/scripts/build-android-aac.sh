#!/usr/bin/env bash
# Cross-compiles the AAC decoders for Android (arm64-v8a, x86_64) as static libraries that
# build.rs links into liblittlesdr.so:
#   FAAD2  (GPL-2.0-or-later, AAC / HE-AAC v1 v2: DAB+)  -> third_party/faad2/<target>/libfaad.a
#   libxaac (Apache-2.0, xHE-AAC / USAC: DRM)            -> third_party/libxaac/build-<target>/libxaacdec.a
# Never FDK-AAC (its licence is incompatible with the GPL).
# Needs: ANDROID_NDK_HOME (default: the app's NDK), cmake, ninja, curl, git.
set -euo pipefail
cd "$(dirname "$0")/.."
ENGINE=$PWD
NDK=${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/29.0.14206865}
API=28
FAAD2_VERSION=2.11.4
TC=$NDK/build/cmake/android.toolchain.cmake
[ -f "$TC" ] || { echo "NDK not found: $NDK" >&2; exit 1; }

# FAAD2 sources (once)
SRC=$ENGINE/third_party/faad2-src
if [ ! -f "$SRC/CMakeLists.txt" ]; then
    mkdir -p "$SRC"
    curl -fsSL "https://github.com/knik0/faad2/archive/refs/tags/$FAAD2_VERSION.tar.gz" | tar xz -C "$SRC" --strip-components=1
fi
# libxaac sources (once), at the revision the engine is tested with
LIBXAAC_REV=2fbadd57bca46693b7a077bcf1513131106d960c
if [ ! -d "$ENGINE/third_party/libxaac/decoder" ]; then
    git clone https://github.com/ittiam-systems/libxaac "$ENGINE/third_party/libxaac"
    git -C "$ENGINE/third_party/libxaac" checkout -q "$LIBXAAC_REV"
fi

for pair in arm64-v8a:aarch64-linux-android x86_64:x86_64-linux-android; do
    ABI=${pair%%:*}
    TARGET=${pair#*:}
    COMMON=(-G Ninja -DCMAKE_TOOLCHAIN_FILE="$TC" -DANDROID_ABI="$ABI" -DANDROID_PLATFORM="android-$API"
            -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON)

    echo "== FAAD2 $ABI"
    B=$ENGINE/third_party/faad2-build/$TARGET
    mkdir -p "$(dirname "$B")"
    cmake -S "$SRC" -B "$B" "${COMMON[@]}" -DBUILD_SHARED_LIBS=OFF -DFAAD_BUILD_CLI=OFF > "$B.log" 2>&1 || { cat "$B.log"; exit 1; }
    cmake --build "$B" --target faad >> "$B.log" 2>&1 || { tail -40 "$B.log"; exit 1; }
    mkdir -p "$ENGINE/third_party/faad2/$TARGET"
    cp "$(find "$B" -name 'libfaad.a' | head -1)" "$ENGINE/third_party/faad2/$TARGET/libfaad.a"

    # FAAD2's DRM build (ER AAC 960-sample frames, DRM SBR / PS): DRM AAC stations. A shared
    # library the engine loads at run time, beside the ordinary FAAD2 linked for DAB+
    echo "== FAAD2 DRM $ABI"
    B=$ENGINE/third_party/faad2-build/$TARGET-drm
    cmake -S "$SRC" -B "$B" "${COMMON[@]}" -DBUILD_SHARED_LIBS=ON -DFAAD_BUILD_CLI=OFF -DCMAKE_PLATFORM_NO_VERSIONED_SONAME=ON > "$B.log" 2>&1 || { cat "$B.log"; exit 1; }
    cmake --build "$B" --target faad_drm >> "$B.log" 2>&1 || { tail -40 "$B.log"; exit 1; }
    mkdir -p "$ENGINE/../kit/jniLibs/$ABI"
    cp "$(find "$B" -name 'libfaad_drm.so' | head -1)" "$ENGINE/../kit/jniLibs/$ABI/libfaad_drm.so"

    echo "== libxaac $ABI"
    B=$ENGINE/third_party/libxaac/build-$TARGET
    cmake -S "$ENGINE/third_party/libxaac" -B "$B" "${COMMON[@]}" > "$B.log" 2>&1 || { cat "$B.log"; exit 1; }
    cmake --build "$B" --target libxaacdec >> "$B.log" 2>&1 || { tail -40 "$B.log"; exit 1; }
done
ls -la "$ENGINE"/third_party/faad2/*/libfaad.a "$ENGINE"/third_party/libxaac/build-*/libxaacdec.a
