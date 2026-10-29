//! Link the explicitly prepared, pinned Windows APM cache. Never download in Cargo.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

fn child(root: &Path, relative: &str) -> PathBuf {
    let relative = Path::new(relative);
    assert!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "Invalid meeting backend cache path"
    );
    let path =
        dunce::canonicalize(root.join(relative)).expect("Prepared meeting backend file missing");
    assert!(
        path.starts_with(root),
        "Meeting backend cache path escapes its root"
    );
    path
}

fn digest(path: &Path) -> String {
    let mut file = File::open(path).expect("Read prepared meeting backend");
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .expect("Hash prepared meeting backend");
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    format!("{:x}", hash.finalize())
}

pub fn build() {
    println!("cargo:rerun-if-env-changed=MEETING_APM_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64")
    {
        return;
    }
    assert_eq!(
        std::env::var("TARGET").as_deref(),
        Ok("x86_64-pc-windows-msvc"),
        "Meeting processing requires the Windows x64 MSVC target"
    );
    let crate_root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let requested = std::env::var_os("MEETING_APM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate_root.join("../../.cache/native/webrtc-apm-2.1-msvc-x64-md"));
    let root = dunce::canonicalize(requested)
        .expect("Run scripts/prepare-meeting-apm.ps1 before building the meeting backend");
    let manifest_path = root.join("manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let bytes = std::fs::read(&manifest_path).expect("Read prepared meeting backend manifest");
    assert!(
        bytes.len() <= 1024 * 1024,
        "Meeting backend manifest too large"
    );
    let manifest: Value = serde_json::from_slice(&bytes).expect("Invalid meeting backend manifest");
    for (key, expected) in [
        ("backend", "webrtc-audio-processing-2.1"),
        ("target", "x86_64-pc-windows-msvc"),
        ("crt", "MD"),
        ("cpp_std", "c++20"),
    ] {
        assert_eq!(
            manifest[key].as_str(),
            Some(expected),
            "Wrong meeting backend {key}"
        );
    }
    let validated = &manifest["validated_build"];
    for machine in ["host", "target"] {
        let actual = &validated["metadata"]["machines"][machine];
        assert_eq!(actual["system"].as_str(), Some("windows"));
        assert_eq!(actual["cpu_family"].as_str(), Some("x86_64"));
    }
    assert_eq!(
        validated["metadata"]["compiler"]["id"].as_str(),
        Some("msvc")
    );
    for (key, expected) in [
        ("b_vscrt", "md"),
        ("buildtype", "release"),
        ("default_library", "static"),
        ("cpp_std", "c++20"),
    ] {
        assert_eq!(
            validated["metadata"]["effective_options"][key].as_str(),
            Some(expected)
        );
    }
    let archive = child(
        &root,
        manifest["archive"].as_str().expect("Missing archive path"),
    );
    println!("cargo:rerun-if-changed={}", archive.display());
    let archive_digest = digest(&archive);
    assert_eq!(
        Some(archive_digest.as_str()),
        manifest["archive_sha256"].as_str(),
        "Meeting backend library changed; rerun the preparation script"
    );
    assert_eq!(
        manifest["archive_sha256"],
        validated["coff"]["archive_sha256"]
    );
    assert_eq!(validated["coff"]["machine"].as_str(), Some("AMD64/0x8664"));
    assert!(validated["coff"]["object_members"]
        .as_u64()
        .is_some_and(|n| n > 0));
    assert_eq!(
        validated["coff"]["object_members"],
        validated["coff"]["release_md_directive_members"]
    );
    assert_eq!(
        std::fs::metadata(&archive).unwrap().len(),
        manifest["archive_bytes"].as_u64().unwrap()
    );
    let include = child(
        &root,
        manifest["include"].as_str().expect("Missing include path"),
    );
    let abseil = child(
        &root,
        manifest["abseil_include"]
            .as_str()
            .expect("Missing Abseil path"),
    );
    println!("cargo:rerun-if-changed=native/meeting_apm");
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .static_crt(false)
        .debug(false)
        .opt_level(2)
        .include(include)
        .include(abseil)
        .file(crate_root.join("native/meeting_apm/meeting_apm_wrapper.cpp"))
        .define("WEBRTC_WIN", None)
        .define("WIN32", None)
        .define("NOMINMAX", None)
        .define("_USE_MATH_DEFINES", None)
        .define("WEBRTC_LIBRARY_IMPL", None)
        .flag_if_supported("/Zc:__cplusplus")
        .flag_if_supported("/utf-8")
        .flag_if_supported("/EHsc")
        .compile("meeting_apm_bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static=webrtc_apm");
    println!("cargo:rustc-link-lib=winmm");
}
