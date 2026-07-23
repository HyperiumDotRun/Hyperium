use std::path::{Path, PathBuf};

pub const PNG: &[u8] = include_bytes!("../assets/icon.png");

pub fn rgba() -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(PNG).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

pub fn rgba_resized(size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(PNG)
        .ok()?
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    Some((img.into_raw(), size, size))
}

pub fn egui_icon() -> Option<eframe::egui::IconData> {
    let (rgba, width, height) = rgba()?;
    Some(eframe::egui::IconData { rgba, width, height })
}

pub fn ensure_ico(config_dir: &Path) -> Option<PathBuf> {
    let img = image::load_from_memory(PNG).ok()?;
    let frame = |size: u32| -> Option<(u32, Vec<u8>)> {
        let resized = img.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
        let mut buf = std::io::Cursor::new(Vec::new());
        resized.write_to(&mut buf, image::ImageFormat::Png).ok()?;
        Some((size, buf.into_inner()))
    };
    let frames: Vec<(u32, Vec<u8>)> = [16, 32, 48, 256].into_iter().filter_map(frame).collect();
    if frames.is_empty() {
        return None;
    }
    let ico = crate::tools::build_ico(&frames);
    let path = config_dir.join("hyperium.ico");
    std::fs::write(&path, ico).ok()?;
    Some(path)
}
