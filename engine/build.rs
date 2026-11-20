// Optional xHE-AAC decoding (DRM): libxaac (Apache-2.0) built in third_party/libxaac/build,
// or LIBXAAC_DIR. Without it the engine builds and DRM audio stays silent.
fn main() {
    println!("cargo:rerun-if-changed=csrc/xaac_shim.c");
    println!("cargo:rerun-if-env-changed=LIBXAAC_DIR");
    println!("cargo::rustc-check-cfg=cfg(xaac)");
    let dir = std::env::var("LIBXAAC_DIR").unwrap_or_else(|_| format!("{}/third_party/libxaac", env!("CARGO_MANIFEST_DIR")));
    let lib = format!("{dir}/build/libxaacdec.a");
    if !std::path::Path::new(&lib).exists() {
        println!("cargo:warning=libxaac not found ({lib}): DRM xHE-AAC audio disabled");
        return;
    }
    cc::Build::new().file("csrc/xaac_shim.c").include(format!("{dir}/common")).include(format!("{dir}/decoder")).warnings(false).compile("xaac_shim");
    println!("cargo:rustc-link-search=native={dir}/build");
    println!("cargo:rustc-link-lib=static=xaacdec");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-cfg=xaac");
}
