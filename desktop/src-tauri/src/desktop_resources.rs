//! Seed the pinned, bundled model into the normal writable model directory.
use std::path::Path;
use tauri::Manager;

pub fn seed_bundled_model(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let source = app.path().resolve(
        "resources/models/sense-voice-int8",
        tauri::path::BaseDirectory::Resource,
    )?;
    if !source.is_dir() {
        // Source-only development can use the existing model download UI.
        return Ok(());
    }
    let models = crate::portable::app_data_dir(app)?.join("models");
    let dest = models.join("sense-voice-int8");
    if dest.join("model.int8.onnx").is_file() && dest.join("tokens.txt").is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(&models)?;
    // A separate staging directory means interrupted first startup cannot
    // advertise a half-written ONNX file as an installed model.
    let staging = models.join("sense-voice-int8.installing");
    std::fs::create_dir_all(&staging)?;
    for name in ["model.int8.onnx", "tokens.txt"] {
        copy_regular(&source.join(name), &staging.join(name))?;
    }
    std::fs::create_dir_all(&dest)?;
    for name in ["model.int8.onnx", "tokens.txt"] {
        std::fs::rename(staging.join(name), dest.join(name))?;
    }
    let _ = std::fs::remove_dir(staging);
    Ok(())
}

fn copy_regular(source: &Path, dest: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "Bundled model must be a regular file",
        ));
    }
    std::fs::copy(source, dest)?;
    Ok(())
}
