use lofty::file::TaggedFileExt;
use std::path::{Path, PathBuf};

pub fn worker_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .thread_name(|i| format!("cover-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

fn decode_cover(bytes: &[u8]) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    Some(reader.decode().ok()?.thumbnail(160, 160).to_rgba8())
}

const MAX_IMAGE_BYTES: u64 = 8_000_000;
const IMAGE_EXTENSIONS: &[&str] = &[
    "bmp", "gif", "ico", "jpg", "jpeg", "png", "pnm", "pbm", "pgm", "ppm", "qoi", "tga", "tif",
    "tiff", "webp",
];

fn normalized_name(name: &str) -> Vec<char> {
    name.chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn name_similarity(left: &str, right: &str) -> f32 {
    let left = normalized_name(left);
    let right = normalized_name(right);
    let longest = left.len().max(right.len());
    if longest == 0 {
        return 0.0;
    }

    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (left_index, left_character) in left.iter().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution_cost = usize::from(left_character != right_character);
            current.push(
                (current[right_index] + 1)
                    .min(previous[right_index + 1] + 1)
                    .min(previous[right_index] + substitution_cost),
            );
        }
        previous = current;
    }

    1.0 - previous[right.len()] as f32 / longest as f32
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

fn best_image_in_directory(directory: &Path) -> Option<image::RgbaImage> {
    let folder_name = directory.file_name()?.to_string_lossy();
    let entries = std::fs::read_dir(directory).ok()?;

    let mut candidates: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || !is_supported_image(&path) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if metadata.len() > MAX_IMAGE_BYTES {
                return None;
            }
            let file_stem = entry.path().file_stem()?.to_string_lossy().into_owned();
            Some((name_similarity(&file_stem, &folder_name), path))
        })
        .collect();
    candidates.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
    });
    candidates.into_iter().find_map(|(_, path)| {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            return None;
        }
        decode_cover(&bytes)
    })
}

fn folder_image_for_track(path: &Path) -> Option<image::RgbaImage> {
    best_image_in_directory(path.parent()?)
}

pub fn rgba_cover(path: String) -> Option<(Vec<u8>, usize, usize)> {
    let track_path = PathBuf::from(path);
    let thumbnail = folder_image_for_track(&track_path).or_else(|| {
        let tagged = lofty::read_from_path(&track_path).ok()?;
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
        let picture = tag.pictures().first()?;
        (picture.data().len() as u64 <= MAX_IMAGE_BYTES)
            .then(|| decode_cover(picture.data()))
            .flatten()
    })?;
    let (width, height) = thumbnail.dimensions();
    Some((thumbnail.into_raw(), width as usize, height as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_name_matching_ignores_case_and_separators() {
        assert_eq!(name_similarity("Summer-Album", "summer album"), 1.0);
    }

    #[test]
    fn exact_folder_name_beats_generic_cover_name() {
        assert!(
            name_similarity("Blue Horizon", "Blue Horizon")
                > name_similarity("cover", "Blue Horizon")
        );
    }

    #[test]
    fn common_photo_extensions_are_supported_case_insensitively() {
        assert!(is_supported_image(Path::new("ARTWORK.PNG")));
        assert!(is_supported_image(Path::new("artwork.jpeg")));
        assert!(is_supported_image(Path::new("artwork.webp")));
        assert!(!is_supported_image(Path::new("track.mp3")));
    }

    #[test]
    fn artwork_is_only_selected_from_the_track_directory() {
        let temporary_root = std::env::temp_dir().join(format!(
            "aura-cover-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let album_directory = temporary_root.join("Album");
        std::fs::create_dir_all(&album_directory).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]))
            .save(temporary_root.join("Album.png"))
            .unwrap();

        let track_path = album_directory.join("song.mp3");
        assert!(folder_image_for_track(&track_path).is_none());

        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 255, 0, 255]))
            .save(album_directory.join("album-cover.png"))
            .unwrap();
        assert!(folder_image_for_track(&track_path).is_some());

        std::fs::remove_dir_all(temporary_root).unwrap();
    }
}
