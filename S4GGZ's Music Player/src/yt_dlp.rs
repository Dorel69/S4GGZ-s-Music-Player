use crate::process_runner::{self, Cancellation};
use reqwest::blocking::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_BINARY_SIZE: u64 = 64 * 1024 * 1024;

fn resolve(cancel: &Cancellation) -> Result<PathBuf, String> {
    let (asset, executable_name) = release_asset()?;
    let directory = dirs::data_local_dir()
        .ok_or("Directorul local de date nu este disponibil")?
        .join("AuraPlayer")
        .join("bin");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let executable = directory.join(executable_name);

    if executable.is_file() {
        return Ok(executable);
    }

    install_release(&directory, &executable, asset, cancel)?;
    Ok(executable)
}

pub fn command_with_cookies(
    cancel: &Cancellation,
    browser: Option<&str>,
) -> Result<Command, String> {
    process_runner::check_cancel(cancel)?;
    let mut command = Command::new(resolve(cancel)?);
    process_runner::check_cancel(cancel)?;
    command.args([
        "--ignore-config",
        "--no-plugin-dirs",
        "--socket-timeout",
        "15",
        "--retries",
        "2",
        "--fragment-retries",
        "2",
    ]);
    if let Some(args) = browser_cookie_args(browser)? {
        command.args(args);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    Ok(command)
}

fn allowed_browser(browser: &str) -> bool {
    matches!(
        browser,
        "chrome" | "edge" | "firefox" | "brave" | "opera" | "vivaldi"
    )
}

fn browser_cookie_args(browser: Option<&str>) -> Result<Option<[&str; 2]>, String> {
    match browser {
        None => Ok(None),
        Some(browser) if allowed_browser(browser) => Ok(Some(["--cookies-from-browser", browser])),
        Some(_) => Err("Browserul selectat pentru cookie-uri nu este acceptat.".into()),
    }
}

pub fn metadata_with_cookies(
    url: &str,
    cancel: &Cancellation,
    browser: Option<&str>,
) -> Result<(String, String), String> {
    let mut command = command_with_cookies(cancel, browser)?;
    command
        .args([
            "--no-warnings",
            "--no-color",
            "--no-playlist",
            "--skip-download",
            "--dump-single-json",
        ])
        .arg("--")
        .arg(url);
    let output = process_runner::run(&mut command, Duration::from_secs(60), cancel, None)?;
    if !output.status.success() {
        return Err(format!(
            "Nu am putut citi metadatele platformei: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Metadatele yt-dlp nu sunt JSON valid: {error}"))?;
    let title = value["title"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("Platforma nu a furnizat titlul melodiei")?
        .to_owned();
    let artist = ["uploader", "channel", "creator", "artist"]
        .iter()
        .find_map(|field| value[*field].as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Unknown Artist")
        .to_owned();
    Ok((title, artist))
}

fn release_asset() -> Result<(&'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok(("yt-dlp.exe", "yt-dlp.exe")),
        _ => Err(format!(
            "Nu există un binar yt-dlp oficial configurat pentru {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )),
    }
}

fn install_release(
    directory: &Path,
    executable: &Path,
    asset: &str,
    cancel: &Cancellation,
) -> Result<(), String> {
    let client = Client::builder()
        .user_agent(concat!("AuraPlayer/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Nu am putut inițializa clientul HTTP: {error}"))?;
    let base_url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
    let binary = download(
        &client,
        &format!("{base_url}/{asset}"),
        MAX_BINARY_SIZE,
        cancel,
    )?;
    let checksums = download(
        &client,
        &format!("{base_url}/SHA2-256SUMS"),
        1024 * 1024,
        cancel,
    )?;
    let checksums = String::from_utf8(checksums)
        .map_err(|error| format!("Fișierul SHA2SUMS nu este UTF-8: {error}"))?;
    let expected = checksum_for(&checksums, asset)?;
    process_runner::check_cancel(cancel)?;
    let actual = format!("{:x}", Sha256::digest(&binary));
    if actual != expected {
        return Err("Verificarea SHA-256 pentru yt-dlp a eșuat".into());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temporary = directory.join(format!(
        "yt-dlp-{}-{timestamp}.download",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Nu am putut crea binarul temporar: {error}"))?;
    if let Err(error) = file.write_all(&binary).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Nu am putut scrie yt-dlp: {error}"));
    }
    drop(file);

    if let Err(error) = fs::rename(&temporary, executable) {
        let _ = fs::remove_file(&temporary);
        if executable.is_file() {
            return Ok(());
        }
        return Err(format!("Nu am putut instala yt-dlp: {error}"));
    }
    Ok(())
}

fn download(
    client: &Client,
    url: &str,
    max_size: u64,
    cancel: &Cancellation,
) -> Result<Vec<u8>, String> {
    process_runner::check_cancel(cancel)?;
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("Descărcarea yt-dlp a eșuat: {error}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > max_size)
    {
        return Err("Fișierul yt-dlp depășește limita de dimensiune".into());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        process_runner::check_cancel(cancel)?;
        let count = response
            .read(&mut chunk)
            .map_err(|error| format!("Nu am putut citi descărcarea yt-dlp: {error}"))?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > max_size {
            return Err("Fișierul yt-dlp depășește limita de dimensiune".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if bytes.len() as u64 > max_size {
        return Err("Fișierul yt-dlp depășește limita de dimensiune".into());
    }
    Ok(bytes)
}

fn checksum_for(checksums: &str, asset: &str) -> Result<String, String> {
    checksums
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let checksum = fields.next()?;
            let filename = fields.next()?.trim_start_matches('*');
            (Path::new(filename).file_name()?.to_str()? == asset)
                .then(|| checksum.to_ascii_lowercase())
        })
        .find(|checksum| {
            checksum.len() == 64 && checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| format!("SHA2SUMS nu conține un checksum valid pentru {asset}"))
}

#[cfg(test)]
mod tests {
    use super::{allowed_browser, browser_cookie_args, checksum_for};

    #[test]
    fn finds_checksum_in_sha256sum_formats() {
        let checksum = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let list = format!("{checksum}  yt-dlp.exe\n{checksum} *other-file");
        assert_eq!(checksum_for(&list, "yt-dlp.exe").unwrap(), checksum);
    }

    #[test]
    fn rejects_missing_or_invalid_checksum() {
        assert!(checksum_for("not-a-checksum  yt-dlp.exe", "yt-dlp.exe").is_err());
        assert!(checksum_for("", "yt-dlp.exe").is_err());
    }

    #[test]
    fn browser_cookie_sources_are_explicitly_allowlisted() {
        for browser in ["chrome", "edge", "firefox", "brave", "opera", "vivaldi"] {
            assert!(allowed_browser(browser));
        }
        for browser in ["", "unknown", "chrome --cookies secret.txt"] {
            assert!(!allowed_browser(browser));
        }
    }

    #[test]
    fn browser_cookie_arguments_are_added_only_for_a_supported_selection() {
        assert_eq!(browser_cookie_args(None).unwrap(), None);
        assert_eq!(
            browser_cookie_args(Some("edge")).unwrap(),
            Some(["--cookies-from-browser", "edge"])
        );
        assert!(browser_cookie_args(Some("unknown")).is_err());
    }
}
