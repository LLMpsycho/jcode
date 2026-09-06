use super::*;

pub(super) fn hash_content(content: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Get PNG dimensions from file
pub(super) fn get_png_dimensions(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read as _;

    let mut header = [0u8; 24];
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log_warn(&format!(
                "Could not open cached Mermaid PNG header: {error}"
            ));
            return None;
        }
    };
    if let Err(error) = file.read_exact(&mut header) {
        log_warn(&format!(
            "Could not read cached Mermaid PNG header: {error}"
        ));
        return None;
    }
    if &header[0..8] == b"\x89PNG\r\n\x1a\n" {
        let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
        let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
        return Some((width, height));
    }
    None
}
