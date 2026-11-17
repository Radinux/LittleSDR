#!/usr/bin/env bash
# Build the native dependencies needed by the SDR++Brown engine (LittleSDR) for Android,
# without Docker. Result: kit/out/<abi>/{include,lib}
#
# Usage: kit/build-kit.sh [abi...]        (default: arm64-v8a x86_64)
# Env:   ANDROID_HOME (default ~/Android/Sdk), NDK_VERSION, CMAKE_VERSION, JOBS
set -euo pipefail

KIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$KIT_DIR/src"
BUILD="$KIT_DIR/build"
OUT="$KIT_DIR/out"

ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
NDK_VERSION="${NDK_VERSION:-29.0.14206865}"
CMAKE_VERSION="${CMAKE_VERSION:-4.1.2}"
API=28
JOBS="${JOBS:-$(nproc)}"
NDK="$ANDROID_HOME/ndk/$NDK_VERSION"
CMAKE="$ANDROID_HOME/cmake/$CMAKE_VERSION/bin/cmake"
NINJA="$ANDROID_HOME/cmake/$CMAKE_VERSION/bin/ninja"

ABIS=("$@")
[ ${#ABIS[@]} -eq 0 ] && ABIS=(arm64-v8a x86_64)

[ -x "$CMAKE" ] || { echo "CMake $CMAKE_VERSION introuvable dans le SDK"; exit 1; }
[ -d "$NDK" ]   || { echo "NDK $NDK_VERSION introuvable"; exit 1; }

# ---------------------------------------------------------------- sources
# Versions are pinned so the kit is reproducible.
ZSTD_VER=1.5.7
FFTW_VER=3.3.10
LIBUSB_VER=1.0.29
VOLK_TAG=v3.2.0
# Forks/commits that provide the "open by file descriptor" APIs used on Android
RTLSDR_REPO=https://github.com/AlexandreRouma/rtl-sdr
HACKRF_REPO=https://github.com/AlexandreRouma/hackrf
AIRSPY_REPO=https://github.com/airspy/airspyone_host
AIRSPYHF_REPO=https://github.com/airspy/airspyhf

fetch_tar() { # url dir
    local url=$1 dir=$2
    [ -d "$SRC/$dir" ] && return
    echo "==> Téléchargement $dir"
    mkdir -p "$SRC/$dir.tmp"
    curl -fsSL "$url" | tar -x -C "$SRC/$dir.tmp" --strip-components=1 -f - ${3:-}
    mv "$SRC/$dir.tmp" "$SRC/$dir"
}
fetch_git() { # repo dir [ref]
    local repo=$1 dir=$2 ref=${3:-}
    [ -d "$SRC/$dir" ] && return
    echo "==> Clonage $dir"
    if [ -n "$ref" ]; then
        git clone -q --depth 1 --branch "$ref" --recurse-submodules --shallow-submodules "$repo" "$SRC/$dir"
    else
        git clone -q --depth 1 --recurse-submodules --shallow-submodules "$repo" "$SRC/$dir"
    fi
}

mkdir -p "$SRC" "$BUILD" "$OUT"
fetch_tar "https://github.com/facebook/zstd/releases/download/v$ZSTD_VER/zstd-$ZSTD_VER.tar.gz" zstd -z
fetch_tar "https://www.fftw.org/fftw-$FFTW_VER.tar.gz" fftw -z
fetch_tar "https://github.com/libusb/libusb/releases/download/v$LIBUSB_VER/libusb-$LIBUSB_VER.tar.bz2" libusb -j
fetch_git https://github.com/gnuradio/volk volk "$VOLK_TAG"
fetch_git "$RTLSDR_REPO" rtl-sdr
fetch_git "$HACKRF_REPO" hackrf
fetch_git "$AIRSPY_REPO" airspyone_host
fetch_git "$AIRSPYHF_REPO" airspyhf

# Record exact revisions used
{
    echo "zstd $ZSTD_VER"; echo "fftw $FFTW_VER"; echo "libusb $LIBUSB_VER"
    for d in volk rtl-sdr hackrf airspyone_host airspyhf; do
        echo "$d $(git -C "$SRC/$d" rev-parse HEAD)"
    done
} > "$OUT/VERSIONS.txt"

# ---------------------------------------------------------------- helpers
cmake_build() { # abi name srcdir [extra cmake args...]
    local abi=$1 name=$2 srcdir=$3; shift 3
    local bdir="$BUILD/$abi/$name"
    mkdir -p "$BUILD/$abi"
    echo "===================== $name ($abi) ====================="
    "$CMAKE" -S "$srcdir" -B "$bdir" -G Ninja -DCMAKE_MAKE_PROGRAM="$NINJA" \
        -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
        -DANDROID_ABI="$abi" -DANDROID_PLATFORM="android-$API" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$OUT/$abi" \
        -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_INSTALL_INCLUDEDIR=include \
        -DCMAKE_FIND_ROOT_PATH="$OUT/$abi" \
        -DCMAKE_PLATFORM_NO_VERSIONED_SONAME=ON \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 -DTHREADS_PTHREAD_ARG=2 \
        -DBUILD_SHARED_LIBS=ON -DENABLE_TESTING=OFF \
        "$@" >"$bdir.log" 2>&1 || { mkdir -p "$(dirname "$bdir")"; tail -40 "$bdir.log"; exit 1; }
    "$CMAKE" --build "$bdir" -j "$JOBS" >>"$bdir.log" 2>&1 || { tail -60 "$bdir.log"; exit 1; }
    "$CMAKE" --install "$bdir" >>"$bdir.log" 2>&1 || { tail -40 "$bdir.log"; exit 1; }
}

libusb_args() { # abi
    echo -DLIBUSB_LIBRARIES="$OUT/$1/lib/libusb1.0.so" -DLIBUSB_INCLUDE_DIRS="$OUT/$1/include" \
         -DLIBUSB_INCLUDE_DIR="$OUT/$1/include" -DLIBUSB_FOUND=1 -DLIBUSB_VERSION=$LIBUSB_VER
}

# ---------------------------------------------------------------- libusb (ndk-build)
build_libusb() {
    echo "===================== libusb (${ABIS[*]}) ====================="
    local log="$BUILD/libusb.log"
    mkdir -p "$BUILD"
    "$NDK/ndk-build" -C "$SRC/libusb/android/jni" -j "$JOBS" \
        APP_ABI="${ABIS[*]}" APP_PLATFORM="android-$API" \
        NDK_OUT="$BUILD/libusb/obj" NDK_LIBS_OUT="$BUILD/libusb/libs" >"$log" 2>&1 || { tail -40 "$log"; exit 1; }
    for abi in "${ABIS[@]}"; do
        mkdir -p "$OUT/$abi/lib" "$OUT/$abi/include"
        cp "$BUILD/libusb/libs/$abi/libusb1.0.so" "$OUT/$abi/lib/"
        cp "$SRC/libusb/libusb/libusb.h" "$OUT/$abi/include/"
        mkdir -p "$OUT/$abi/include/libusb-1.0"
        cp "$SRC/libusb/libusb/libusb.h" "$OUT/$abi/include/libusb-1.0/"
    done
}

# ---------------------------------------------------------------- per ABI
build_abi() {
    local abi=$1
    local simd=()
    case $abi in
        x86_64) simd=(-DENABLE_SSE=ON -DENABLE_SSE2=ON) ;;
        arm64-v8a) simd=(-DENABLE_NEON=ON) ;;
    esac

    cmake_build "$abi" zstd "$SRC/zstd/build/cmake" \
        -DZSTD_BUILD_PROGRAMS=OFF -DZSTD_BUILD_STATIC=OFF -DZSTD_BUILD_TESTS=OFF -DZSTD_LEGACY_SUPPORT=OFF

    cmake_build "$abi" fftw "$SRC/fftw" -DENABLE_FLOAT=ON -DENABLE_THREADS=ON -DBUILD_TESTS=OFF "${simd[@]}"

    cmake_build "$abi" volk "$SRC/volk" -DENABLE_TESTING=OFF -DENABLE_MODTOOL=OFF \
        -DVOLK_CPU_FEATURES=ON -DENABLE_PROFILING=OFF

    cmake_build "$abi" rtl-sdr "$SRC/rtl-sdr" $(libusb_args "$abi") \
        -DBUILD_UTILITIES=OFF -DINSTALL_UDEV_RULES=OFF -DDETACH_KERNEL_DRIVER=OFF

    cmake_build "$abi" airspy "$SRC/airspyone_host" $(libusb_args "$abi") -DINSTALL_UDEV_RULES=OFF

    cmake_build "$abi" airspyhf "$SRC/airspyhf" $(libusb_args "$abi") -DINSTALL_UDEV_RULES=OFF

    cmake_build "$abi" hackrf "$SRC/hackrf/host/libhackrf" $(libusb_args "$abi") \
        -DDISABLE_USB_DEVICE_DISCOVERY=ON -DINSTALL_UDEV_RULES=OFF
}

build_libusb
for abi in "${ABIS[@]}"; do build_abi "$abi"; done

# Runtime .so files in the layout Gradle expects (jniLibs/<abi>/*.so)
for abi in "${ABIS[@]}"; do
    mkdir -p "$KIT_DIR/jniLibs/$abi"
    cp "$OUT/$abi"/lib/*.so "$KIT_DIR/jniLibs/$abi/"
done

echo
echo "Kit prêt dans $OUT"
for abi in "${ABIS[@]}"; do echo "  $abi: $(ls "$OUT/$abi/lib" | grep '\.so$' | tr '\n' ' ')"; done
