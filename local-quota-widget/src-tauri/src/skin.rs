use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, ImageFormat, ImageReader, Limits};
use std::{
    fs::{self, File},
    io::{BufReader, Cursor, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

const MAX_SOURCE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_DECODE_ALLOC: u64 = 96 * 1024 * 1024;
const NORMALIZED_IMAGE_EDGE: u32 = 2048;
const MAX_STORED_BYTES: u64 = 12 * 1024 * 1024;

fn skin_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("skins").join("custom-skin.jpg"))
        .map_err(|error| format!("无法定位照片皮肤目录：{error}"))
}

fn image_reader(bytes: &[u8]) -> Result<ImageReader<BufReader<Cursor<&[u8]>>>, String> {
    ImageReader::new(BufReader::new(Cursor::new(bytes)))
        .with_guessed_format()
        .map_err(|error| format!("无法识别图片格式：{error}"))
}

fn normalize_image(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let format = image_reader(bytes)?
        .format()
        .ok_or_else(|| "无法识别图片格式".to_string())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("仅支持 PNG、JPG、WEBP 图片".into());
    }

    let (width, height) = image_reader(bytes)?
        .into_dimensions()
        .map_err(|error| format!("无法读取图片尺寸：{error}"))?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "图片尺寸无效".to_string())?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err("照片尺寸不能超过 4096×4096 或 1600 万像素".into());
    }

    let mut reader = image_reader(bytes)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("照片已损坏或无法解码：{error}"))?;
    let normalized = if width > NORMALIZED_IMAGE_EDGE || height > NORMALIZED_IMAGE_EDGE {
        image.resize(
            NORMALIZED_IMAGE_EDGE,
            NORMALIZED_IMAGE_EDGE,
            FilterType::Lanczos3,
        )
    } else {
        image
    }
    .to_rgb8();

    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 88)
        .encode_image(&normalized)
        .map_err(|error| format!("无法转换照片：{error}"))?;
    if encoded.len() as u64 > MAX_STORED_BYTES {
        return Err("处理后的照片仍然过大".into());
    }
    Ok(encoded)
}

fn read_selected_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("无法读取所选照片：{error}"))?;
    if !metadata.is_file() {
        return Err("请选择一个普通图片文件".into());
    }
    if metadata.len() == 0 || metadata.len() > MAX_SOURCE_BYTES {
        return Err("照片需小于 10 MB".into());
    }
    fs::read(path).map_err(|error| format!("无法读取所选照片：{error}"))
}

fn persist_skin(app: &AppHandle, bytes: &[u8]) -> Result<(), String> {
    let destination = skin_path(app)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "照片皮肤目录无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建照片皮肤目录：{error}"))?;
    let temporary = parent.join("custom-skin.tmp");
    let mut file =
        File::create(&temporary).map_err(|error| format!("无法保存照片皮肤：{error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("无法保存照片皮肤：{error}"))?;
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| format!("无法替换照片皮肤：{error}"))?;
    }
    fs::rename(&temporary, &destination).map_err(|error| format!("无法保存照片皮肤：{error}"))
}

fn skin_data_url(bytes: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn choose_custom_skin(app: AppHandle) -> Result<Option<String>, String> {
    let selected = app
        .dialog()
        .file()
        .set_title("选择照片皮肤")
        .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|error| format!("无法读取所选路径：{error}"))?;
    let source = read_selected_file(&path)?;
    let normalized = normalize_image(&source)?;
    persist_skin(&app, &normalized)?;
    Ok(Some(skin_data_url(&normalized)))
}

#[tauri::command]
pub fn get_custom_skin(app: AppHandle) -> Result<Option<String>, String> {
    let path = skin_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata =
        fs::metadata(&path).map_err(|error| format!("无法读取已保存的照片皮肤：{error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_STORED_BYTES {
        return Err("已保存的照片皮肤无效".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("无法读取已保存的照片皮肤：{error}"))?;
    Ok(Some(skin_data_url(&bytes)))
}

#[tauri::command]
pub fn clear_custom_skin(app: AppHandle) -> Result<(), String> {
    let path = skin_path(&app)?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("无法删除照片皮肤：{error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat};

    fn small_png() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(12, 8)
            .write_to(&mut output, ImageFormat::Png)
            .expect("test PNG should encode");
        output.into_inner()
    }

    #[test]
    fn normalizes_supported_image_to_metadata_free_jpeg() {
        let encoded = normalize_image(&small_png()).expect("valid PNG should normalize");
        assert!(encoded.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(encoded.len() < MAX_STORED_BYTES as usize);
    }

    #[test]
    fn rejects_unknown_or_broken_images() {
        assert!(normalize_image(b"not an image").is_err());
    }
}
