use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Aqua,
    Beer,
    Disco,
}

impl Appearance {
    pub fn label(self) -> &'static str {
        match self {
            Self::Aqua => "Aqua",
            Self::Beer => "Beer",
            Self::Disco => "Disco",
        }
    }
    pub fn background_file(self) -> &'static str {
        match self {
            Self::Aqua => "bg.png",
            Self::Beer => "beer-bg.png",
            Self::Disco => "disco-bg.png",
        }
    }
    pub fn particle_file(self) -> &'static str {
        match self {
            Self::Aqua => "bubble.png",
            Self::Beer => "beer.png",
            Self::Disco => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoMode {
    #[default]
    Party,
    Moving,
    Static,
}

impl DiscoMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Party => "Party Mode",
            Self::Moving => "În mișcare",
            Self::Static => "Statică",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeerFrequency {
    Rare,
    #[default]
    Medium,
    Frequent,
}

impl BeerFrequency {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rare => "M-am pocait",
            Self::Medium => "Doar o bere",
            Self::Frequent => "Hai la bere",
        }
    }
    pub fn interval(self) -> (f64, f64) {
        match self {
            Self::Rare => (12.0, 20.0),
            Self::Medium => (5.0, 10.0),
            Self::Frequent => (3.0, 6.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDensity {
    #[default]
    Many,
    Medium,
    Few,
}

impl EffectDensity {
    pub fn label(self, fish: bool) -> &'static str {
        match (self, fish) {
            (Self::Many, true) => "Mulți",
            (Self::Many, false) => "Multe",
            (Self::Medium, _) => "Mediu",
            (Self::Few, true) => "Puțini",
            (Self::Few, false) => "Puține",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub equalizer: crate::equalizer::Equalizer,
    pub youtube_cookie_browser: Option<String>,
    pub appearance: Appearance,
    pub beer_frequency: BeerFrequency,
    pub fish_enabled: bool,
    pub bubbles_enabled: bool,
    pub fish_density: EffectDensity,
    pub bubble_density: EffectDensity,
    pub background: Option<PathBuf>,
    pub beer_background: Option<PathBuf>,
    pub disco_ball_enabled: bool,
    pub disco_floor_enabled: bool,
    pub disco_mode: DiscoMode,
    pub disco_background: Option<PathBuf>,
    #[serde(deserialize_with = "reflection_percent")]
    pub disco_window_reflection_percent: u8,
    #[serde(deserialize_with = "ray_count")]
    pub disco_party_rays: u16,
    #[serde(deserialize_with = "ray_count")]
    pub disco_moving_rays: u16,
    #[serde(deserialize_with = "ray_count")]
    pub disco_static_rays: u16,
}

fn reflection_percent<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    u8::deserialize(deserializer).map(|value| value.min(100))
}

fn ray_count<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    u16::deserialize(deserializer).map(|value| value.min(256))
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            equalizer: crate::equalizer::Equalizer::default(),
            youtube_cookie_browser: None,
            appearance: Appearance::Aqua,
            beer_frequency: BeerFrequency::Medium,
            fish_enabled: true,
            bubbles_enabled: true,
            fish_density: EffectDensity::Many,
            bubble_density: EffectDensity::Many,
            background: None,
            beer_background: None,
            disco_ball_enabled: true,
            disco_floor_enabled: true,
            disco_mode: DiscoMode::Party,
            disco_background: None,
            disco_window_reflection_percent: 18,
            disco_party_rays: 144,
            disco_moving_rays: 108,
            disco_static_rays: 72,
        }
    }
}

impl AppSettings {
    pub fn disco_ray_count(&self, mode: DiscoMode) -> u16 {
        match mode {
            DiscoMode::Party => self.disco_party_rays,
            DiscoMode::Moving => self.disco_moving_rays,
            DiscoMode::Static => self.disco_static_rays,
        }
        .min(256)
    }

    pub fn set_disco_ray_count(&mut self, mode: DiscoMode, count: u16) {
        let count = count.min(256);
        match mode {
            DiscoMode::Party => self.disco_party_rays = count,
            DiscoMode::Moving => self.disco_moving_rays = count,
            DiscoMode::Static => self.disco_static_rays = count,
        }
    }

    pub fn active_background(&self) -> Option<&Path> {
        match self.appearance {
            Appearance::Aqua => self.background.as_deref(),
            Appearance::Beer => self.beer_background.as_deref(),
            Appearance::Disco => self.disco_background.as_deref(),
        }
    }

    pub fn set_background(&mut self, path: Option<PathBuf>) {
        match self.appearance {
            Appearance::Aqua => self.background = path,
            Appearance::Beer => self.beer_background = path,
            Appearance::Disco => self.disco_background = path,
        }
    }
    pub fn user_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|root| root.join("AuraPlayer").join("settings.json"))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("Nu am putut citi setările: {error}")),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("Nu am putut citi setările: {error}")),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or("Folderul setărilor nu este disponibil")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let temporary = parent.join(format!("settings-{}.tmp", std::process::id()));
        fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Nu am putut salva setările: {error}"));
        }
        Ok(())
    }
}

pub fn background_image(path: &Path) -> Result<image::RgbaImage, String> {
    let image = image::ImageReader::open(path)
        .map_err(|error| format!("Nu am putut deschide imaginea: {error}"))?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| format!("Imaginea nu poate fi citită: {error}"))?;
    if image.width() > 3840 || image.height() > 3840 {
        Ok(image.thumbnail(3840, 3840).to_rgba8())
    } else {
        Ok(image.to_rgba8())
    }
}

/// Keep one managed copy; the photo selected by the user is never modified.
pub fn import_background(
    source: &Path,
    directory: &Path,
) -> Result<(PathBuf, image::RgbaImage), String> {
    let image = background_image(source)?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let destination = directory.join("background.png");
    let temporary = directory.join(format!("background-{}.tmp", std::process::id()));
    image
        .save_with_format(&temporary, image::ImageFormat::Png)
        .map_err(|error| format!("Nu am putut salva fundalul: {error}"))?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Nu am putut salva fundalul: {error}"));
    }
    Ok((destination, image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestLibrary;

    #[test]
    fn preferences_survive_repeated_saves_and_missing_fields_keep_defaults() {
        let library = TestLibrary::new("preferences");
        let path = library.directory.join("settings.json");
        assert_eq!(AppSettings::load(&path).unwrap(), AppSettings::default());
        let mut settings = AppSettings {
            appearance: Appearance::Beer,
            youtube_cookie_browser: Some("edge".into()),
            beer_frequency: BeerFrequency::Frequent,
            beer_background: Some(library.directory.join("Beer").join("background.png")),
            fish_enabled: false,
            bubbles_enabled: false,
            fish_density: EffectDensity::Medium,
            bubble_density: EffectDensity::Few,
            background: Some(library.directory.join("poză de fundal.png")),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path).unwrap(), settings);
        settings.fish_enabled = true;
        settings.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path).unwrap(), settings);
        fs::write(&path, r#"{"fish_enabled":false}"#).unwrap();
        let settings = AppSettings::load(&path).unwrap();
        assert!(!settings.fish_enabled);
        assert!(settings.bubbles_enabled);
        assert!(settings.background.is_none());
        assert_eq!(settings.fish_density, EffectDensity::Many);
        assert_eq!(settings.bubble_density, EffectDensity::Many);
        assert_eq!(settings.appearance, Appearance::Aqua);
        assert_eq!(settings.beer_frequency, BeerFrequency::Medium);
        assert!(settings.youtube_cookie_browser.is_none());
        assert!(settings.beer_background.is_none());
        assert!(settings.disco_ball_enabled && settings.disco_floor_enabled);
        assert_eq!(settings.disco_mode, DiscoMode::Party);
        assert!(settings.disco_background.is_none());
        assert_eq!(settings.disco_window_reflection_percent, 18);
        assert_eq!(settings.disco_ray_count(DiscoMode::Party), 144);
        assert_eq!(settings.disco_ray_count(DiscoMode::Moving), 108);
        assert_eq!(settings.disco_ray_count(DiscoMode::Static), 72);
        fs::write(&path, "invalid JSON").unwrap();
        assert!(AppSettings::load(&path).is_err());
    }

    #[test]
    fn each_theme_retains_its_own_background_and_reset_affects_only_active_theme() {
        let mut settings = AppSettings::default();
        settings.set_background(Some(PathBuf::from("aqua-photo.png")));
        settings.appearance = Appearance::Beer;
        assert!(settings.active_background().is_none());
        settings.set_background(Some(PathBuf::from("beer-photo.png")));
        assert_eq!(
            settings.active_background(),
            Some(Path::new("beer-photo.png"))
        );
        settings.appearance = Appearance::Disco;
        assert!(settings.active_background().is_none());
        settings.set_background(Some(PathBuf::from("disco-photo.png")));
        assert_eq!(
            settings.active_background(),
            Some(Path::new("disco-photo.png"))
        );
        settings.set_background(None);
        assert!(settings.active_background().is_none());
        settings.appearance = Appearance::Beer;
        assert_eq!(
            settings.active_background(),
            Some(Path::new("beer-photo.png"))
        );
        settings.appearance = Appearance::Aqua;
        assert_eq!(
            settings.active_background(),
            Some(Path::new("aqua-photo.png"))
        );
    }

    #[test]
    fn disco_mode_effects_and_photo_survive_restart() {
        let library = TestLibrary::new("disco-preferences");
        let path = library.directory.join("settings.json");
        let mut preferences = AppSettings {
            appearance: Appearance::Disco,
            disco_ball_enabled: false,
            disco_floor_enabled: false,
            disco_background: Some(library.directory.join("Disco").join("background.png")),
            ..Default::default()
        };
        for mode in [DiscoMode::Party, DiscoMode::Moving, DiscoMode::Static] {
            preferences.disco_mode = mode;
            preferences.save(&path).unwrap();
            assert_eq!(AppSettings::load(&path).unwrap(), preferences);
        }
    }

    #[test]
    fn disco_ray_counts_are_independent_persistent_and_clamped_to_the_slider_range() {
        let library = TestLibrary::new("disco-ray-counts");
        let path = library.directory.join("settings.json");
        fs::write(&path, r#"{"appearance":"disco"}"#).unwrap();
        let mut preferences = AppSettings::load(&path).unwrap();
        assert_eq!(
            (
                preferences.disco_party_rays,
                preferences.disco_moving_rays,
                preferences.disco_static_rays
            ),
            (144, 108, 72)
        );

        preferences.set_disco_ray_count(DiscoMode::Party, 0);
        preferences.set_disco_ray_count(DiscoMode::Moving, 256);
        preferences.set_disco_ray_count(DiscoMode::Static, 17);
        assert_eq!(preferences.disco_ray_count(DiscoMode::Party), 0);
        assert_eq!(preferences.disco_ray_count(DiscoMode::Moving), 256);
        assert_eq!(preferences.disco_ray_count(DiscoMode::Static), 17);
        preferences.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path).unwrap(), preferences);

        preferences.set_disco_ray_count(DiscoMode::Static, u16::MAX);
        assert_eq!(preferences.disco_static_rays, 256);
        assert_eq!(preferences.disco_ray_count(DiscoMode::Party), 0);
        assert_eq!(preferences.disco_ray_count(DiscoMode::Moving), 256);
        preferences.disco_party_rays = u16::MAX;
        assert_eq!(preferences.disco_ray_count(DiscoMode::Party), 256);

        fs::write(
            &path,
            r#"{"disco_party_rays":65535,"disco_moving_rays":257,"disco_static_rays":0}"#,
        )
        .unwrap();
        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(
            (
                loaded.disco_party_rays,
                loaded.disco_moving_rays,
                loaded.disco_static_rays
            ),
            (256, 256, 0)
        );
    }

    #[test]
    fn disco_window_reflection_probability_defaults_persists_and_clamps_loaded_values() {
        let library = TestLibrary::new("disco-window-reflections");
        let path = library.directory.join("settings.json");
        fs::write(&path, r#"{"appearance":"disco"}"#).unwrap();
        let mut preferences = AppSettings::load(&path).unwrap();
        assert_eq!(preferences.disco_window_reflection_percent, 18);
        for percent in [0, 18, 100] {
            preferences.disco_window_reflection_percent = percent;
            preferences.save(&path).unwrap();
            assert_eq!(AppSettings::load(&path).unwrap(), preferences);
        }
        fs::write(&path, r#"{"disco_window_reflection_percent":255}"#).unwrap();
        assert_eq!(
            AppSettings::load(&path)
                .unwrap()
                .disco_window_reflection_percent,
            100
        );
    }

    #[test]
    fn custom_background_is_copied_replaced_and_validated_without_touching_source() {
        let library = TestLibrary::new("custom-background");
        let source = library.directory.join("original.png");
        let directory = library.directory.join("preferences");
        image::RgbaImage::from_pixel(80, 40, image::Rgba([20, 80, 160, 255]))
            .save(&source)
            .unwrap();
        let original = fs::read(&source).unwrap();
        let (saved, pixels) = import_background(&source, &directory).unwrap();
        assert_eq!(pixels.dimensions(), (80, 40));
        assert_eq!(fs::read(&source).unwrap(), original);
        fs::remove_file(&source).unwrap();
        assert_eq!(background_image(&saved).unwrap(), pixels);
        image::RgbaImage::from_pixel(25, 50, image::Rgba([90, 30, 50, 255]))
            .save(&source)
            .unwrap();
        let (replacement, pixels) = import_background(&source, &directory).unwrap();
        assert_eq!(replacement, saved);
        assert_eq!(pixels.dimensions(), (25, 50));
        assert_eq!(background_image(&saved).unwrap(), pixels);
        fs::write(&source, b"invalid image").unwrap();
        assert!(import_background(&source, &directory).is_err());
        assert_eq!(background_image(&saved).unwrap(), pixels);
    }
}
