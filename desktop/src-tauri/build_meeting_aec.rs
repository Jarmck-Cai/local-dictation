//! Minimal static C build; no downloaded build-time code or additional runtime.

pub fn build() {
    let root = std::path::Path::new("native/speexdsp-1.2.1");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut build = cc::Build::new();
    build
        .include(root.join("include"))
        .include(root.join("libspeexdsp"))
        .file(root.join("libspeexdsp/mdf.c"))
        .file(root.join("libspeexdsp/fftwrap.c"))
        .file(root.join("libspeexdsp/smallft.c"))
        .define("FLOATING_POINT", None)
        .define("USE_SMALLFT", None)
        .define("EXPORT", Some(""))
        .warnings(false);
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.define("inline", Some("__inline"));
    }
    build.compile("meeting_speexdsp");
}
