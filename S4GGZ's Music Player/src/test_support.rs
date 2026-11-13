use crate::database::Database;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) struct TestLibrary {
    pub directory: PathBuf,
    pub music: PathBuf,
    pub db: Database,
}

impl TestLibrary {
    pub fn new(name: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-data");
        fs::create_dir_all(&parent).unwrap();
        let directory = parent.join(format!("aura-{name}-{}-{timestamp}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let music = directory.join("Music");
        fs::create_dir(&music).unwrap();
        let db = Database::new(directory.join("library.sqlite3")).unwrap();
        Self {
            directory,
            music,
            db,
        }
    }

    pub fn mp3(&self, artist: &str, title: &str) -> PathBuf {
        let path = self.music.join(artist).join(format!("{title}.mp3"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_test_mp3(&path);
        path
    }
}

impl Drop for TestLibrary {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

/// MPEG-1 Layer III frames containing silence, with no copyrighted audio.
pub(crate) fn write_test_mp3(path: &Path) {
    let mut frame = [0_u8; 417];
    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
    fs::write(path, frame.repeat(20)).unwrap();
}

pub(crate) fn wait_for_scans(app: &mut crate::AuraApp, ctx: &eframe::egui::Context) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.music_scan_running
        || app.scan_running
        || app.refresh_running
        || app.refresh_pending.is_some()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Background scan did not finish"
        );
        app.poll_events(ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Headless UI tests deliberately discard GPU uploads while retaining shapes.
pub(crate) fn run_ui(
    ctx: &eframe::egui::Context,
    input: eframe::egui::RawInput,
    callback: impl FnMut(&mut eframe::egui::Ui),
) -> eframe::egui::FullOutput {
    let mut output = ctx.run_ui(input, callback);
    output.textures_delta.clear();
    output
}
