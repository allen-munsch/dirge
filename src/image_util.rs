use std::path::Path;

use base64::Engine;
use image::GenericImageView;

#[allow(dead_code)]
pub fn load_image_base64(path: &Path) -> Result<String, String> {
    let img = image::open(path).map_err(|e| format!("Failed to open image: {e}"))?;
    let (width, height) = img.dimensions();
    let pixels = (width as u64) * (height as u64);

    if pixels > 8_000_000 {
        return Err(format!(
            "Image too large: {}x{} ({} pixels, max 8M)",
            width, height, pixels
        ));
    }

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode PNG: {e}"))?;

    let mut b64 = String::new();
    base64::engine::general_purpose::STANDARD.encode_string(&buf, &mut b64);

    if b64.len() > 20_000_000 {
        return Err("Encoded image exceeds 20MB limit".to_string());
    }

    Ok(b64)
}

#[allow(dead_code)]
pub fn is_image_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_image_path() {
        assert!(is_image_path("photo.png"));
        assert!(is_image_path("photo.jpg"));
        assert!(is_image_path("photo.JPEG"));
        assert!(is_image_path("photo.gif"));
        assert!(is_image_path("photo.webp"));
        assert!(!is_image_path("photo.txt"));
        assert!(!is_image_path("photo.md"));
    }
}
