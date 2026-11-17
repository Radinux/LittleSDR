cmake_minimum_required(VERSION 3.22)
project(littlesdr C CXX)

# ---------------------------------------------------------------------------
# Paths
get_filename_component(REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../.." ABSOLUTE)
set(SDRPP_ROOT "${REPO_ROOT}/third_party/sdrpp-brown")
if (NOT SDR_KIT_ROOT)
    set(SDR_KIT_ROOT "${REPO_ROOT}/kit/out")
endif ()
set(KIT "${SDR_KIT_ROOT}/${ANDROID_ABI}")
if (NOT EXISTS "${KIT}/lib/libvolk.so")
    message(FATAL_ERROR "Kit introuvable pour ${ANDROID_ABI}: lance kit/build-kit.sh")
endif ()

set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
if (NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif ()

# Variables consumed by SDR++Brown's module CMake files (sdrpp_module.cmake)
set(SDRPP_MODULE_CMAKE "${SDRPP_ROOT}/sdrpp_module.cmake")
set(SDRPP_CORE_ROOT "${SDRPP_ROOT}/core/src/")
set(SDRPP_COMPILER_FLAGS -O3 -std=c++17 -Wno-unused-command-line-argument -Wno-deprecated-register)
set(SDRPP_MODULE_COMPILER_FLAGS ${SDRPP_COMPILER_FLAGS})
add_compile_options(-Wno-deprecated-declarations -Wno-format-security -Wno-unused-result)

# ---------------------------------------------------------------------------
# SDR++Brown core (DSP, signal path, config, module manager...) + headless backend
add_subdirectory("${SDRPP_ROOT}/core/libcorrect" libcorrect EXCLUDE_FROM_ALL)

file(GLOB_RECURSE CORE_SRC
    "${SDRPP_ROOT}/core/src/*.cpp" "${SDRPP_ROOT}/core/src/*.c"
    "${SDRPP_ROOT}/misc_modules/noise_reduction_logmmse/src/omlsa_mcra/*.cpp"
    "${SDRPP_ROOT}/misc_modules/noise_reduction_logmmse/src/omlsa_mcra.cpp")
list(FILTER CORE_SRC EXCLUDE REGEX "sdrpp_ft8_mshv_main.cpp")

add_library(sdrpp_core SHARED ${CORE_SRC} headless_backend.cpp)
target_compile_definitions(sdrpp_core PRIVATE SDRPP_IS_CORE FLOG_ANDROID_TAG="LittleSDR")
target_compile_definitions(sdrpp_core PUBLIC INSTALL_PREFIX="/")
target_compile_options(sdrpp_core PRIVATE ${SDRPP_COMPILER_FLAGS})
target_include_directories(sdrpp_core PUBLIC
    "${SDRPP_ROOT}/core/src"
    "${SDRPP_ROOT}/core/src/imgui"
    "${SDRPP_ROOT}/core/backends/android"   # android_backend.h (API we implement)
    "${SDRPP_ROOT}/core/libcorrect/include"
    "${CMAKE_CURRENT_SOURCE_DIR}"
    "${KIT}/include")
target_link_libraries(sdrpp_core PUBLIC
    "${KIT}/lib/libvolk.so"
    "${KIT}/lib/libfftw3f.so"
    "${KIT}/lib/libzstd.so"
    correct_static
    android EGL GLESv3 log)

# ---------------------------------------------------------------------------
# Engine modules (loaded at runtime with dlopen, same as upstream)
set(LITTLESDR_MODULES
    source_modules/rtl_sdr_source
    source_modules/airspy_source
    source_modules/airspyhf_source
    source_modules/hackrf_source
    source_modules/rtl_tcp_source
    source_modules/spyserver_source
    source_modules/kiwisdr_source
    decoder_modules/radio)
foreach (mod ${LITTLESDR_MODULES})
    get_filename_component(modname "${mod}" NAME)
    add_subdirectory("${SDRPP_ROOT}/${mod}" "modules/${modname}")
endforeach ()

# Extra entry points compiled into radio.so (advanced demodulator options)
target_sources(radio PRIVATE modules/radio_ext.cpp)

# Our own AAudio sink (upstream android_audio_sink needs NativeActivity)
add_library(audio_sink SHARED modules/aaudio_sink/main.cpp)
set_target_properties(audio_sink PROPERTIES PREFIX "")
target_compile_options(audio_sink PRIVATE ${SDRPP_MODULE_COMPILER_FLAGS})
target_link_libraries(audio_sink PRIVATE sdrpp_core aaudio)

# ---------------------------------------------------------------------------
# Shaders -> SPIR-V headers (glslc ships with the NDK)
find_program(GLSLC glslc HINTS "${ANDROID_NDK}/shader-tools/${ANDROID_HOST_TAG}" REQUIRED)
set(SHADER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/shaders")
set(SHADER_OUT "${CMAKE_CURRENT_BINARY_DIR}/shaders")
file(MAKE_DIRECTORY "${SHADER_OUT}")
set(SHADER_HEADERS)
foreach (shader waterfall.vert waterfall.frag spectrum.vert spectrum.frag)
    string(REPLACE "." "_" sym "${shader}")
    set(out "${SHADER_OUT}/${sym}.spv.h")
    add_custom_command(OUTPUT "${out}"
        COMMAND "${GLSLC}" -O --target-env=vulkan1.0 -mfmt=num -o "${out}" "${SHADER_DIR}/${shader}"
        DEPENDS "${SHADER_DIR}/${shader}" "${SHADER_DIR}/common.glsl"
        COMMENT "glslc ${shader}")
    list(APPEND SHADER_HEADERS "${out}")
endforeach ()

# ---------------------------------------------------------------------------
# JNI bridge + Vulkan spectrum/waterfall renderer
add_library(littlesdr SHARED
    jni_bridge.cpp
    engine.cpp
    smgui_json.cpp
    spectrum_renderer.cpp
    ${SHADER_HEADERS})
target_include_directories(littlesdr PRIVATE "${SHADER_OUT}" "${SDRPP_ROOT}/decoder_modules/radio/src")
target_compile_options(littlesdr PRIVATE -O2 -Wall -Wno-unused-parameter)
target_link_libraries(littlesdr PRIVATE sdrpp_core vulkan android log)
