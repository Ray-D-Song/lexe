use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use lexe_core::VERSION;

use crate::executable::{
    self, parse_sha256sums_entry, sha256_hex, Platform, DEFAULT_RELEASE_BASE_URL,
};

pub const DEFAULT_LATEST_RELEASE_URL: &str = "https://github.com/Ray-D-Song/lexe/releases/latest";

type UpgradeError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpgradeArgs {
    pub version: Option<String>,
    pub dry_run: bool,
    pub force: bool,
    pub help: bool,
}

struct TempFileGuard {
    path: Option<PathBuf>,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn print_upgrade_usage() {
    println!(
        r#"lexe v{VERSION}

Usage:
  lexe upgrade [options]

Options:
  --version <ver>   Install a specific release version (e.g. 0.2.0 or v0.2.0)
  --dry-run         Check target version without downloading or modifying binary
  -f, --force       Reinstall even if already running the target version
  -h, --help        Print this help message
"#
    );
}

pub fn normalize_version(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);

    if stripped.is_empty()
        || stripped.contains("..")
        || !stripped
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        || !stripped
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
    {
        return Err(format!("Invalid version string: '{raw}'"));
    }

    Ok(stripped.to_string())
}

pub fn parse_upgrade_args(args: &[String]) -> Result<UpgradeArgs, String> {
    let mut parsed = UpgradeArgs::default();
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => {
                parsed.help = true;
            },
            "--dry-run" => {
                parsed.dry_run = true;
            },
            "-f" | "--force" => {
                parsed.force = true;
            },
            "--version" => {
                if parsed.version.is_some() {
                    return Err("Multiple --version values specified".to_string());
                }
                let next = args
                    .get(i + 1)
                    .ok_or_else(|| "--version requires a value".to_string())?;
                parsed.version = Some(normalize_version(next)?);
                i += 1;
            },
            _ if arg.starts_with("--version=") => {
                if parsed.version.is_some() {
                    return Err("Multiple --version values specified".to_string());
                }
                let val = arg.trim_start_matches("--version=");
                parsed.version = Some(normalize_version(val)?);
            },
            _ if arg.starts_with('-') => {
                return Err(format!("Unknown option: {arg}"));
            },
            _ => {
                return Err(format!(
                    "Unexpected argument '{arg}'. Use 'lexe upgrade --version <ver>' to target a specific version."
                ));
            },
        }
        i += 1;
    }

    Ok(parsed)
}

pub fn parse_release_tag_from_location(location: &str) -> Result<String, String> {
    let clean = location.split(['?', '#']).next().unwrap_or(location);
    let clean = clean.trim_end_matches('/');
    let (_, tag) = clean
        .rsplit_once("/releases/tag/")
        .ok_or_else(|| format!("Unexpected release redirect Location header: '{location}'"))?;
    if tag.is_empty() || tag.contains('/') {
        return Err(format!(
            "Invalid release tag in Location header: '{location}'"
        ));
    }
    normalize_version(tag)
}

fn is_npm_managed_with(exe_path: &Path, managed_by: Option<&str>) -> bool {
    if managed_by
        .map(|v| v.trim().eq_ignore_ascii_case("npm"))
        .unwrap_or(false)
    {
        return true;
    }
    exe_path
        .components()
        .any(|c| matches!(c, Component::Normal(seg) if seg == "node_modules"))
}

pub fn is_npm_managed(exe_path: &Path) -> bool {
    is_npm_managed_with(exe_path, env::var("LEXE_MANAGED_BY").ok().as_deref())
}

fn verify_staged_binary(
    staged_path: &Path,
    expected_version: Option<&str>,
) -> Result<String, UpgradeError> {
    let output = Command::new(staged_path)
        .arg("--version")
        .output()
        .map_err(|e| {
            format!(
                "Failed to execute staged binary '{}' for verification: {e}",
                staged_path.display()
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "Staged binary '{}' exited with status {} during --version check",
            staged_path.display(),
            output.status
        )
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.starts_with("lexe v") {
        return Err(format!("Unexpected --version output from staged binary: '{stdout}'").into());
    }

    if let Some(ver) = expected_version {
        let exact = format!("lexe v{ver}");
        let prefix = format!("lexe v{ver} ");
        if stdout != exact && !stdout.starts_with(&prefix) {
            return Err(
                format!("Staged binary version mismatch: expected v{ver}, got '{stdout}'").into(),
            );
        }
    }

    Ok(stdout)
}

fn replace_binary(staged_path: &Path, target_exe: &Path) -> Result<(), UpgradeError> {
    #[cfg(not(windows))]
    {
        fs::rename(staged_path, target_exe).map_err(|e| {
            format!(
                "Failed to replace '{}' with '{}': {e}",
                target_exe.display(),
                staged_path.display()
            )
        })?;
        Ok(())
    }

    #[cfg(windows)]
    {
        let mut backup_os = target_exe.as_os_str().to_os_string();
        backup_os.push(".old");
        let backup_path = PathBuf::from(backup_os);
        let _ = fs::remove_file(&backup_path);
        fs::rename(target_exe, &backup_path).map_err(|e| {
            format!(
                "Failed to move existing binary '{}' before replacement: {e}",
                target_exe.display()
            )
        })?;
        if let Err(err) = fs::rename(staged_path, target_exe) {
            let _ = fs::rename(&backup_path, target_exe);
            return Err(format!(
                "Failed to install new binary '{}' (restored previous binary): {err}",
                target_exe.display()
            )
            .into());
        }
        let _ = fs::remove_file(&backup_path);
        Ok(())
    }
}

fn write_staged_file(staged_path: &Path, bytes: &[u8]) -> Result<(), UpgradeError> {
    if let Err(err) = fs::write(staged_path, bytes) {
        if err.kind() == ErrorKind::PermissionDenied {
            let dir = staged_path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string());
            return Err(format!(
                "Permission denied writing temporary binary in {dir}. Try running with elevated permissions (e.g. sudo): {err}"
            )
            .into());
        }
        return Err(err.into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(staged_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(staged_path, perms)?;
    }

    Ok(())
}

async fn resolve_target_location(
    args: &UpgradeArgs,
    latest_url: &str,
    release_download_base: &str,
    release_base_override: Option<&str>,
) -> Result<(Option<String>, String), UpgradeError> {
    if let Some(custom) = release_base_override
        .map(|s| s.trim().trim_end_matches('/'))
        .filter(|s| !s.is_empty())
    {
        let inferred_version = args.version.clone().or_else(|| {
            custom
                .rsplit('/')
                .next()
                .filter(|seg| seg.starts_with('v') || seg.starts_with('V'))
                .and_then(|seg| normalize_version(seg).ok())
        });
        return Ok((inferred_version, custom.to_string()));
    }

    let download_base = release_download_base.trim_end_matches('/');
    if let Some(ver) = &args.version {
        return Ok((Some(ver.clone()), format!("{download_base}/v{ver}")));
    }

    let location = executable::fetch_redirect_location(latest_url).await?;
    let ver = parse_release_tag_from_location(&location)?;
    Ok((Some(ver.clone()), format!("{download_base}/v{ver}")))
}

async fn run_upgrade_inner(
    args: &UpgradeArgs,
    target_exe: &Path,
    current_version: &str,
    latest_url: &str,
    release_download_base: &str,
    release_base_override: Option<&str>,
) -> Result<(), UpgradeError> {
    let (target_version, base_url) = resolve_target_location(
        args,
        latest_url,
        release_download_base,
        release_base_override,
    )
    .await?;

    if !args.force && target_version.as_deref() == Some(current_version) {
        println!("lexe v{current_version} is already up to date.");
        return Ok(());
    }

    if args.dry_run {
        match &target_version {
            Some(ver) => println!(
                "Would upgrade lexe from v{current_version} to v{ver} ({})",
                target_exe.display()
            ),
            None => println!(
                "Would upgrade lexe from v{current_version} using {base_url} ({})",
                target_exe.display()
            ),
        }
        return Ok(());
    }

    let asset = Platform::current().release_asset_name();
    let sums_url = format!("{base_url}/SHA256SUMS");
    eprintln!("Fetching checksums from {sums_url}...");
    let sums_bytes = executable::fetch_url_bytes(&sums_url).await?;
    let sums_text = String::from_utf8(sums_bytes)
        .map_err(|e| format!("Invalid UTF-8 in SHA256SUMS from {sums_url}: {e}"))?;
    let expected_sha = parse_sha256sums_entry(&sums_text, asset)
        .map_err(|e| format!("Checksum lookup failed for {asset}: {e}"))?;

    let asset_url = format!("{base_url}/{asset}");
    eprintln!("Downloading {asset} from {asset_url}...");
    let bytes = executable::fetch_url_bytes(&asset_url).await?;
    let actual_sha = sha256_hex(&bytes);
    if actual_sha != expected_sha {
        return Err(format!(
            "SHA256 verification failed for {asset}: expected {expected_sha}, got {actual_sha}"
        )
        .into());
    }
    eprintln!("Verified SHA256 for {asset}");

    let parent_dir = target_exe
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            format!(
                "Cannot determine parent directory of '{}'",
                target_exe.display()
            )
        })?;
    let staged_path = parent_dir.join(format!(".lexe-upgrade.{}.tmp", std::process::id()));
    let mut guard = TempFileGuard::new(staged_path.clone());

    write_staged_file(&staged_path, &bytes)?;
    let verified_version = verify_staged_binary(&staged_path, target_version.as_deref())?;
    replace_binary(&staged_path, target_exe)?;
    guard.disarm();

    match target_version {
        Some(ver) => println!(
            "Upgraded lexe from v{current_version} to v{ver} ({})",
            target_exe.display()
        ),
        None => println!(
            "Upgraded lexe from v{current_version} to {verified_version} ({})",
            target_exe.display()
        ),
    }

    Ok(())
}

pub async fn run_upgrade(args: &UpgradeArgs) -> Result<(), UpgradeError> {
    if executable::extract_embedded_bytecode().is_some() {
        return Err(
            "Self-upgrade is not supported inside a packaged application executable.".into(),
        );
    }

    let current_exe = env::current_exe()?;
    let target_exe = fs::canonicalize(&current_exe).unwrap_or_else(|_| current_exe.clone());
    if is_npm_managed(&current_exe) || is_npm_managed(&target_exe) {
        return Err(
            "This lexe binary appears to be managed by npm. Run 'npm install -g lexe@latest' to upgrade."
                .into(),
        );
    }

    let override_url = env::var("LEXE_RELEASE_BASE_URL").ok();
    run_upgrade_inner(
        args,
        &target_exe,
        VERSION,
        DEFAULT_LATEST_RELEASE_URL,
        DEFAULT_RELEASE_BASE_URL,
        override_url.as_deref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn test_normalize_version() {
        assert_eq!(normalize_version("0.2.0").unwrap(), "0.2.0");
        assert_eq!(normalize_version("v0.2.0").unwrap(), "0.2.0");
        assert_eq!(
            normalize_version(" V1.0.0-alpha.1 ").unwrap(),
            "1.0.0-alpha.1"
        );
        assert!(normalize_version("").is_err());
        assert!(normalize_version("v").is_err());
        assert!(normalize_version("../0.2.0").is_err());
        assert!(normalize_version("0.2.0/evil").is_err());
        assert!(normalize_version("0.2.0 beta").is_err());
    }

    #[test]
    fn test_parse_upgrade_args() {
        assert_eq!(parse_upgrade_args(&[]).unwrap(), UpgradeArgs::default());

        let parsed = parse_upgrade_args(&[
            "--version".to_string(),
            "v0.2.0".to_string(),
            "--dry-run".to_string(),
            "-f".to_string(),
        ])
        .unwrap();
        assert_eq!(
            parsed,
            UpgradeArgs {
                version: Some("0.2.0".to_string()),
                dry_run: true,
                force: true,
                help: false,
            }
        );

        let eq_parsed =
            parse_upgrade_args(&["--version=0.3.0".to_string(), "--force".to_string()]).unwrap();
        assert_eq!(eq_parsed.version.as_deref(), Some("0.3.0"));
        assert!(eq_parsed.force);

        assert!(parse_upgrade_args(&["--help".to_string()]).unwrap().help);
        assert!(parse_upgrade_args(&["--version".to_string()]).is_err());
        assert!(parse_upgrade_args(&["--unknown".to_string()]).is_err());
        assert!(parse_upgrade_args(&["0.2.0".to_string()]).is_err());
        assert!(parse_upgrade_args(&[
            "--version=0.1.0".to_string(),
            "--version".to_string(),
            "0.2.0".to_string(),
        ])
        .is_err());
    }

    #[test]
    fn test_parse_release_tag_from_location() {
        assert_eq!(
            parse_release_tag_from_location(
                "https://github.com/Ray-D-Song/lexe/releases/tag/v0.2.0"
            )
            .unwrap(),
            "0.2.0"
        );
        assert_eq!(
            parse_release_tag_from_location(
                "https://github.com/Ray-D-Song/lexe/releases/tag/v0.2.0-rc.1?foo=bar#top/"
            )
            .unwrap(),
            "0.2.0-rc.1"
        );
        assert!(
            parse_release_tag_from_location("https://github.com/Ray-D-Song/lexe/releases").is_err()
        );
        assert!(parse_release_tag_from_location(
            "https://github.com/Ray-D-Song/lexe/releases/tag/"
        )
        .is_err());
    }

    #[test]
    fn test_is_npm_managed_detection() {
        assert!(is_npm_managed_with(
            Path::new("/tmp/project/node_modules/lexe/vendor/lexe-macos-arm64"),
            None
        ));
        assert!(is_npm_managed_with(
            Path::new("/usr/local/bin/lexe"),
            Some("npm")
        ));
        assert!(!is_npm_managed_with(
            Path::new("/Users/dev/.local/bin/lexe"),
            None
        ));
    }

    async fn spawn_upgrade_mock_server(
        latest_tag: String,
        asset_name: String,
        asset_bytes: Vec<u8>,
        sums_body: String,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tag_path = format!("/releases/tag/{latest_tag}");
        let sums_path = format!("/releases/download/{latest_tag}/SHA256SUMS");
        let bin_path = format!("/releases/download/{latest_tag}/{asset_name}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let tag_path = tag_path.clone();
                let sums_path = sums_path.clone();
                let bin_path = bin_path.clone();
                let sums_body = sums_body.clone();
                let asset_bytes = asset_bytes.clone();

                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");

                    if path == "/releases/latest" {
                        let resp = format!(
                            "HTTP/1.1 302 Found\r\nLocation: {tag_path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = socket.write_all(resp.as_bytes()).await;
                    } else if path == sums_path {
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            sums_body.len(),
                            sums_body
                        );
                        let _ = socket.write_all(resp.as_bytes()).await;
                    } else if path == bin_path {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            asset_bytes.len()
                        );
                        let _ = socket.write_all(header.as_bytes()).await;
                        let _ = socket.write_all(&asset_bytes).await;
                    } else {
                        let _ = socket
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });

        (format!("http://{addr}"), handle)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_upgrade_e2e_latest_dry_run_up_to_date_replace_and_force() {
        let tmp_dir = env::temp_dir().join(format!("lexe-upgrade-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();

        let target_exe = tmp_dir.join("lexe");
        fs::write(&target_exe, b"#!/bin/sh\necho 'lexe v0.1.0 (test, arch)'\n").unwrap();

        let asset = Platform::current().release_asset_name().to_string();
        let new_payload = b"#!/bin/sh\necho 'lexe v0.2.0 (test, arch)'\n".to_vec();
        let digest = sha256_hex(&new_payload);
        let sums = format!("{digest}  {asset}\n");

        let (base_http, server) =
            spawn_upgrade_mock_server("v0.2.0".to_string(), asset, new_payload.clone(), sums).await;
        let latest_url = format!("{base_http}/releases/latest");
        let download_base = format!("{base_http}/releases/download");

        // 1. Same version skips cleanly without touching target
        run_upgrade_inner(
            &UpgradeArgs {
                version: Some("0.1.0".to_string()),
                dry_run: false,
                force: false,
                help: false,
            },
            &target_exe,
            "0.1.0",
            &latest_url,
            &download_base,
            None,
        )
        .await
        .unwrap();
        assert!(fs::read(&target_exe)
            .unwrap()
            .starts_with(b"#!/bin/sh\necho 'lexe v0.1.0"));

        // 2. --dry-run resolves latest without modifying binary
        run_upgrade_inner(
            &UpgradeArgs {
                version: None,
                dry_run: true,
                force: false,
                help: false,
            },
            &target_exe,
            "0.1.0",
            &latest_url,
            &download_base,
            None,
        )
        .await
        .unwrap();
        assert!(fs::read(&target_exe)
            .unwrap()
            .starts_with(b"#!/bin/sh\necho 'lexe v0.1.0"));

        // 3. Real latest upgrade replaces binary, preserves +x, and passes --version check
        run_upgrade_inner(
            &UpgradeArgs::default(),
            &target_exe,
            "0.1.0",
            &latest_url,
            &download_base,
            None,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&target_exe).unwrap(), new_payload);
        let output = Command::new(&target_exe).arg("--version").output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "lexe v0.2.0 (test, arch)"
        );

        // 4. --force reinstalls even when already on v0.2.0
        run_upgrade_inner(
            &UpgradeArgs {
                version: Some("0.2.0".to_string()),
                dry_run: false,
                force: true,
                help: false,
            },
            &target_exe,
            "0.2.0",
            &latest_url,
            &download_base,
            None,
        )
        .await
        .unwrap();

        server.abort();
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_upgrade_rejects_sha256_mismatch_without_touching_target() {
        let tmp_dir = env::temp_dir().join(format!("lexe-upgrade-badsha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();

        let target_exe = tmp_dir.join("lexe");
        let original = b"original-binary".to_vec();
        fs::write(&target_exe, &original).unwrap();

        let asset = Platform::current().release_asset_name().to_string();
        let sums = format!("{}  {}\n", "0".repeat(64), asset);

        let (base_http, server) = spawn_upgrade_mock_server(
            "v0.2.0".to_string(),
            asset,
            b"corrupt-binary".to_vec(),
            sums,
        )
        .await;
        let latest_url = format!("{base_http}/releases/latest");
        let download_base = format!("{base_http}/releases/download");

        let err = run_upgrade_inner(
            &UpgradeArgs::default(),
            &target_exe,
            "0.1.0",
            &latest_url,
            &download_base,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("SHA256 verification failed"));
        assert_eq!(fs::read(&target_exe).unwrap(), original);
        let staged = tmp_dir.join(format!(".lexe-upgrade.{}.tmp", std::process::id()));
        assert!(!staged.exists());

        server.abort();
        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
