//! Managed album pictures live beside the launcher, independently of music files.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: u64 = 32 * 1024 * 1024;

pub fn directory() -> PathBuf {
    let executable = std::env::current_exe().ok();
    let parent = executable.as_deref().and_then(Path::parent);
    let main = parent
        .and_then(|parent| {
            parent
                .ancestors()
                .take(5)
                .find(|directory| directory.join("S4GGZ.exe").is_file())
        })
        .or(parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    main.join("AlbumArt")
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    images: HashMap<i64, String>,
}

pub struct Store {
    directory: PathBuf,
    index: Index,
}

fn safe_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    !name.contains(['/', '\\', ':'])
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

impl Store {
    pub fn open(directory: PathBuf) -> Result<Self, String> {
        let index = match fs::read(directory.join("index.json")) {
            Ok(bytes) => serde_json::from_slice::<Index>(&bytes)
                .map_err(|error| format!("Nu pot citi indexul coperților: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(error) => return Err(format!("Nu pot citi coperțile: {error}")),
        };
        if index.images.values().any(|name| !safe_name(name)) {
            return Err("Indexul coperților conține o cale invalidă".into());
        }
        Ok(Self { directory, index })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn path(&self, album: i64) -> Option<PathBuf> {
        self.index
            .images
            .get(&album)
            .map(|name| self.directory.join(name))
            .filter(|path| path.is_file())
    }
    pub fn register(&mut self, album: i64, name: String) -> Result<(), String> {
        if !safe_name(&name) || !self.directory.join(&name).is_file() {
            return Err("Fișierul copertei nu este disponibil".into());
        }
        let previous = self.index.images.insert(album, name.clone());
        let result = (|| {
            fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
            let bytes =
                serde_json::to_vec_pretty(&self.index).map_err(|error| error.to_string())?;
            let temporary = self
                .directory
                .join(format!("index-{}.tmp", std::process::id()));
            fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
            fs::rename(&temporary, self.directory.join("index.json"))
                .map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            if let Some(previous) = previous {
                self.index.images.insert(album, previous);
            } else {
                self.index.images.remove(&album);
            }
            return Err(format!("Nu pot salva coperta albumului: {error}"));
        }
        if let Some(old) = previous.filter(|old| old != &name) {
            // Only files created by this module can be removed when replaced.
            if old.starts_with(&format!("album-{album}-"))
                && !self.index.images.values().any(|value| value == &old)
            {
                let _ = fs::remove_file(self.directory.join(old));
            }
        }
        Ok(())
    }
}

fn decode(bytes: &[u8]) -> Result<(image::DynamicImage, image::ImageFormat), String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let format = reader.format().ok_or("Format de imagine necunoscut")?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    Ok((
        reader
            .decode()
            .map_err(|error| format!("Imagine invalidă sau prea mare: {error}"))?,
        format,
    ))
}

pub fn import(directory: &Path, album: i64, source: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(source)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Imaginea depășește limita de 32 MB".into());
    }
    let (_, format) = decode(&bytes)?;
    let extension = format
        .extensions_str()
        .first()
        .ok_or("Format de imagine nesuportat")?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let name = format!("album-{album}-{}.{}", &digest[..24], extension);
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let destination = directory.join(&name);
    if !destination.is_file() {
        let temporary = directory.join(format!("{name}.tmp"));
        fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &destination).map_err(|error| error.to_string())?;
    }
    Ok(name)
}

pub fn rgba(path: &Path) -> Option<(Vec<u8>, usize, usize)> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let image = decode(&bytes).ok()?.0.thumbnail(192, 192).to_rgba8();
    let (width, height) = image.dimensions();
    Some((image.into_raw(), width as usize, height as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestLibrary;
    #[test]
    fn imported_picture_is_copied_persists_and_replaces_only_its_album() {
        let library = TestLibrary::new("album-pictures");
        let source = library.directory.join("original.png");
        let directory = library.directory.join("AlbumArt");
        image::RgbaImage::from_pixel(16, 12, image::Rgba([255, 0, 0, 255]))
            .save(&source)
            .unwrap();
        let original = fs::read(&source).unwrap();
        let mut store = Store::open(directory.clone()).unwrap();
        let name = import(&directory, 12, &source).unwrap();
        store.register(12, name).unwrap();
        let old = store.path(12).unwrap();
        assert_eq!(fs::read(&old).unwrap(), original);
        store
            .register(13, import(&directory, 13, &source).unwrap())
            .unwrap();
        image::RgbaImage::from_pixel(16, 12, image::Rgba([0, 255, 0, 255]))
            .save(&source)
            .unwrap();
        store
            .register(12, import(&directory, 12, &source).unwrap())
            .unwrap();
        assert!(!old.exists());
        fs::remove_file(source).unwrap();
        let reopened = Store::open(directory).unwrap();
        assert_eq!(
            &rgba(&reopened.path(12).unwrap()).unwrap().0[..4],
            &[0, 255, 0, 255]
        );
        assert_eq!(
            &rgba(&reopened.path(13).unwrap()).unwrap().0[..4],
            &[255, 0, 0, 255]
        );
    }
    #[test]
    fn invalid_images_and_paths_do_not_replace_an_existing_picture() {
        let library = TestLibrary::new("album-invalid-picture");
        let directory = library.directory.join("AlbumArt");
        let invalid = library.directory.join("invalid.png");
        fs::write(&invalid, b"not an image").unwrap();
        assert!(import(&directory, 1, &invalid).is_err());
        let mut store = Store::open(directory.clone()).unwrap();
        assert!(store.register(1, "../outside.png".into()).is_err());
        assert!(store.path(1).is_none());
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("index.json"),
            br#"{"images":{"1":"../outside.png"}}"#,
        )
        .unwrap();
        assert!(Store::open(directory).is_err());
    }
}
