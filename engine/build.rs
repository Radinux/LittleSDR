// Optional xHE-AAC decoding (DRM): libxaac (Apache-2.0) built in third_party/libxaac/build,
// or LIBXAAC_DIR. Without it the engine builds and DRM audio stays silent.
fn main() {
    shaders();
    println!("cargo:rerun-if-changed=csrc/xaac_shim.c");
    println!("cargo:rerun-if-env-changed=LIBXAAC_DIR");
    println!("cargo::rustc-check-cfg=cfg(xaac)");
    println!("cargo::rustc-check-cfg=cfg(faad)");
    let target = std::env::var("TARGET").unwrap_or_default();
    let android = target.contains("android");
    let manifest = env!("CARGO_MANIFEST_DIR");
    // FAAD2: the system's on the desktop; on Android a build for the target, if present
    if !android {
        println!("cargo:rustc-cfg=faad");
    } else if let Ok(d) = std::env::var("FAAD2_DIR").map(|d| format!("{d}/{target}")).or_else(|_| Ok::<_, ()>(format!("{manifest}/third_party/faad2/{target}"))) {
        if std::path::Path::new(&format!("{d}/libfaad.a")).exists() {
            println!("cargo:rustc-link-search=native={d}");
            println!("cargo:rustc-cfg=faad");
        } else {
            println!("cargo:warning=FAAD2 not built for {target}: DAB+ audio disabled");
        }
    }
    // librtlsdr for Android: the app's prebuilt one (kit/jniLibs/<abi>)
    if android {
        let abi = if target.starts_with("aarch64") { "arm64-v8a" } else { "x86_64" };
        let d = std::env::var("RTLSDR_LIB_DIR").unwrap_or_else(|_| format!("{manifest}/../kit/jniLibs/{abi}"));
        println!("cargo:rustc-link-search=native={d}");
    }
    let dir = std::env::var("LIBXAAC_DIR").unwrap_or_else(|_| format!("{manifest}/third_party/libxaac"));
    let lib = if android { format!("{dir}/build-{target}/libxaacdec.a") } else { format!("{dir}/build/libxaacdec.a") };
    if !std::path::Path::new(&lib).exists() {
        println!("cargo:warning=libxaac not found ({lib}): DRM xHE-AAC audio disabled");
        return;
    }
    cc::Build::new().file("csrc/xaac_shim.c").include(format!("{dir}/common")).include(format!("{dir}/decoder")).warnings(false).compile("xaac_shim");
    println!("cargo:rustc-link-search=native={}", std::path::Path::new(&lib).parent().unwrap().display());
    println!("cargo:rustc-link-lib=static=xaacdec");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-cfg=xaac");
}

/// Android: the renderer's GLSL shaders to SPIR-V with the NDK's glslc
fn shaders() {
    if !std::env::var("TARGET").unwrap_or_default().contains("android") {
        return;
    }
    println!("cargo:rerun-if-changed=shaders");
    let ndk = std::env::var("ANDROID_NDK_HOME").or_else(|_| std::env::var("ANDROID_NDK_ROOT")).or_else(|_| std::env::var("CARGO_NDK_ANDROID_NDK")).expect("ANDROID_NDK_HOME");
    let glslc = format!("{ndk}/shader-tools/linux-x86_64/glslc");
    let out = std::env::var("OUT_DIR").unwrap();
    for sh in ["waterfall.vert", "waterfall.frag", "spectrum.vert", "spectrum.frag"] {
        let st = std::process::Command::new(&glslc)
            .args(["-O", "--target-env=vulkan1.0", "-o", &format!("{out}/{}.spv", sh.replace('.', "_")), &format!("shaders/{sh}")])
            .status()
            .expect("glslc");
        assert!(st.success(), "glslc {sh}");
    }
}
