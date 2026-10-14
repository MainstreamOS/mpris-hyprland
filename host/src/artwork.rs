//! Turn bounded embedded artwork into standard file URLs for MPRIS clients.
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_DATA_URL: usize = 256 * 1024;
static SERIAL: AtomicU64 = AtomicU64::new(0);

fn decode(url: &str) -> Option<(Vec<u8>, &'static str)> {
    if url.len() > MAX_DATA_URL {
        return None;
    }
    let (header, payload) = url.split_once(',')?;
    let header = header.to_ascii_lowercase();
    let mime = header.split(';').next()?;
    let extension = match mime {
        "data:image/png" => "png",
        "data:image/jpeg" => "jpg",
        "data:image/webp" => "webp",
        "data:image/gif" => "gif",
        "data:image/avif" => "avif",
        "data:image/svg+xml" => "svg",
        "data:image/bmp" => "bmp",
        "data:image/x-icon" | "data:image/vnd.microsoft.icon" => "ico",
        _ => return None,
    };
    let mut raw = Vec::with_capacity(payload.len());
    let mut chars = payload.bytes();
    while let Some(c) = chars.next() {
        raw.push(if c == b'%' {
            let h = (chars.next()? as char).to_digit(16)?;
            let l = (chars.next()? as char).to_digit(16)?;
            (h * 16 + l) as u8
        } else {
            c
        });
    }
    let bytes = if header.split(';').any(|part| part == "base64") {
        STANDARD.decode(raw).ok()?
    } else {
        raw
    };
    if bytes.is_empty() {
        return None;
    }
    Some((bytes, extension))
}

fn cache(url: &str, directory: &Path) -> Option<String> {
    let (bytes, extension) = decode(url)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .ok()?;
    let path = directory.join(format!("{digest}.{extension}"));
    if !path.is_file() {
        let temp = directory.join(format!(
            ".{}.{}.part",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(&bytes)?;
            fs::rename(&temp, &path)
        })();
        let _ = fs::remove_file(temp);
        result.ok()?;
    }
    let mut encoded = String::from("file://");
    for b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(b) {
            encoded.push(*b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    Some(encoded)
}

pub fn resolve(url: &str) -> String {
    if !url
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case("data:"))
    {
        return url.to_string();
    }
    let directory = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".cache")));
    directory
        .and_then(|p| cache(url, &p.join("mpris-hyprland/artwork")))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_base64_and_percent_encoded_images_only() {
        assert_eq!(
            decode("data:image/png;base64,aGVsbG8=").unwrap().0,
            b"hello"
        );
        assert_eq!(
            decode("data:image/svg+xml,%3Csvg%3E%3C%2Fsvg%3E")
                .unwrap()
                .0,
            b"<svg></svg>"
        );
        for bad in [
            "data:text/html;base64,aGVsbG8=",
            "data:image/png;base64,??",
            "data:image/png,",
            "data:image/png,%GG",
        ] {
            assert!(decode(bad).is_none(), "{bad}");
        }
        assert!(decode(&format!("data:image/png,{}", "x".repeat(MAX_DATA_URL))).is_none());
    }
    #[test]
    fn embedded_art_is_cached_atomically_and_url_encoded() {
        let dir = std::env::temp_dir().join(format!("mpris art-test-{}", std::process::id()));
        let art = "data:image/png;base64,aGVsbG8=";
        let url = cache(art, &dir).unwrap();
        assert!(url.starts_with("file:///"));
        assert!(url.contains("%20"));
        assert_eq!(cache(art, &dir).unwrap(), url);
        let files: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(
            fs::read(files[0].as_ref().unwrap().path()).unwrap(),
            b"hello"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
