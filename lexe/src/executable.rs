use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use lexe_core::compiler::compile_file_to_bytes;
use lexe_core::VERSION;
use sha2::{Digest, Sha256};

pub const SECTION_NAME: &str = "1exec0de";
pub const DEFAULT_RELEASE_BASE_URL: &str = "https://github.com/Ray-D-Song/lexe/releases/download";

type ExecError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Dynamically linked against glibc (glibc 2.28 baseline). Supports N-API native addons.
    LinuxX64Gnu,
    LinuxArm64Gnu,
    /// Statically linked against musl. Does not support N-API native addons.
    LinuxX64Musl,
    LinuxArm64Musl,
    DarwinX64,
    DarwinArm64,
    WindowsX64,
    WindowsArm64,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            match (cfg!(target_arch = "x86_64"), cfg!(target_env = "musl")) {
                (true, false) => Platform::LinuxX64Gnu,
                (true, true) => Platform::LinuxX64Musl,
                (false, false) => Platform::LinuxArm64Gnu,
                (false, true) => Platform::LinuxArm64Musl,
            }
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "x86_64") {
                Platform::DarwinX64
            } else {
                Platform::DarwinArm64
            }
        } else if cfg!(target_arch = "aarch64") {
            Platform::WindowsArm64
        } else {
            Platform::WindowsX64
        }
    }

    /// Name used by `-p/--platform` and in `-p` output file names.
    /// The glibc variants keep the historical `linux-x64` / `linux-arm64` names.
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::LinuxX64Gnu => "linux-x64",
            Platform::LinuxArm64Gnu => "linux-arm64",
            Platform::LinuxX64Musl => "linux-x64-musl",
            Platform::LinuxArm64Musl => "linux-arm64-musl",
            Platform::DarwinX64 => "darwin-x64",
            Platform::DarwinArm64 => "darwin-arm64",
            Platform::WindowsX64 => "windows-x64",
            Platform::WindowsArm64 => "windows-arm64",
        }
    }

    pub fn release_asset_name(self) -> &'static str {
        match self {
            Platform::LinuxX64Gnu => "lexe-linux-x64-gnu",
            Platform::LinuxArm64Gnu => "lexe-linux-arm64-gnu",
            Platform::LinuxX64Musl => "lexe-linux-x64-musl",
            Platform::LinuxArm64Musl => "lexe-linux-arm64-musl",
            Platform::DarwinX64 => "lexe-macos-x64",
            Platform::DarwinArm64 => "lexe-macos-arm64",
            Platform::WindowsX64 => "lexe-windows-x64.exe",
            Platform::WindowsArm64 => "lexe-windows-arm64.exe",
        }
    }

    pub fn is_windows(self) -> bool {
        matches!(self, Platform::WindowsX64 | Platform::WindowsArm64)
    }

    pub fn all() -> &'static [Platform] {
        &[
            Platform::LinuxX64Gnu,
            Platform::LinuxArm64Gnu,
            Platform::LinuxX64Musl,
            Platform::LinuxArm64Musl,
            Platform::DarwinX64,
            Platform::DarwinArm64,
            Platform::WindowsX64,
            Platform::WindowsArm64,
        ]
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "linux-x64" | "linux-x64-gnu" => Ok(Platform::LinuxX64Gnu),
            "linux-arm64" | "linux-arm64-gnu" => Ok(Platform::LinuxArm64Gnu),
            "linux-x64-musl" => Ok(Platform::LinuxX64Musl),
            "linux-arm64-musl" => Ok(Platform::LinuxArm64Musl),
            "darwin-x64" | "macos-x64" => Ok(Platform::DarwinX64),
            "darwin-arm64" | "macos-arm64" => Ok(Platform::DarwinArm64),
            "windows-x64" => Ok(Platform::WindowsX64),
            "windows-arm64" => Ok(Platform::WindowsArm64),
            other => Err(format!(
                "Unknown platform '{other}'. Valid options: linux-x64, linux-arm64, linux-x64-musl, linux-arm64-musl, darwin-x64, darwin-arm64, windows-x64, windows-arm64"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileArgs {
    pub input: PathBuf,
    pub output: Option<String>,
    pub outdir: Option<PathBuf>,
    pub platforms: Vec<Platform>,
    pub create_executable: bool,
    pub runtime_dir: Option<PathBuf>,
    pub sea_config: Option<PathBuf>,
    pub assets: Vec<(String, PathBuf)>,
    pub assets_dir: Option<PathBuf>,
    pub sea_mode: bool,
}

fn take_flag_value(
    args: &[String],
    idx: &mut usize,
    flag: &str,
    inline: Option<&str>,
) -> Result<String, String> {
    if let Some(val) = inline {
        if val.is_empty() {
            return Err(format!("{flag} requires a non-empty value"));
        }
        return Ok(val.to_string());
    }
    if *idx + 1 >= args.len() {
        return Err(format!("{flag} requires a value"));
    }
    *idx += 1;
    Ok(args[*idx].clone())
}

pub fn parse_compile_args(args: &[String]) -> Result<CompileArgs, String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<String> = None;
    let mut outdir: Option<PathBuf> = None;
    let mut platforms: Vec<Platform> = Vec::new();
    let mut create_executable = false;
    let mut runtime_dir: Option<PathBuf> = None;
    let mut sea_config: Option<PathBuf> = None;
    let mut assets: Vec<(String, PathBuf)> = Vec::new();
    let mut assets_dir: Option<PathBuf> = None;
    let mut sea_mode = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--executable" {
            create_executable = true;
            i += 1;
            continue;
        }
        if arg == "--sea" {
            sea_mode = true;
            i += 1;
            continue;
        }

        if let Some((flag, inline)) = split_flag(arg) {
            match flag {
                "-i" | "--input" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    input = Some(PathBuf::from(val));
                },
                "-o" | "--output" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    output = Some(val);
                },
                "-d" | "--outdir" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    outdir = Some(PathBuf::from(val));
                },
                "-p" | "--platform" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    for item in val.split(',') {
                        if item.trim().is_empty() {
                            continue;
                        }
                        let parsed = item.parse::<Platform>()?;
                        if !platforms.contains(&parsed) {
                            platforms.push(parsed);
                        }
                    }
                    if platforms.is_empty() {
                        return Err(format!("{flag} requires at least one platform"));
                    }
                },
                "--runtime-dir" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    runtime_dir = Some(PathBuf::from(val));
                },
                "--sea-config" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    sea_config = Some(PathBuf::from(val));
                    sea_mode = true;
                },
                "--asset" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    let Some((key, file_path)) = val.split_once('=') else {
                        return Err(format!(
                            "--asset must be formatted as <key>=<path>, got '{val}'"
                        ));
                    };
                    if key.is_empty() || file_path.is_empty() {
                        return Err(format!("--asset requires non-empty key and path: '{val}'"));
                    }
                    assets.push((key.to_string(), PathBuf::from(file_path)));
                    sea_mode = true;
                },
                "--assets-dir" => {
                    let val = take_flag_value(args, &mut i, flag, inline)?;
                    assets_dir = Some(PathBuf::from(val));
                    sea_mode = true;
                },
                other => {
                    return Err(format!("Unknown compile option: {other}"));
                },
            }
            i += 1;
            continue;
        }

        if arg.starts_with('-') {
            return Err(format!("Unknown compile option: {arg}"));
        }

        if input.is_none() {
            input = Some(PathBuf::from(arg));
        } else if output.is_none() {
            output = Some(arg.to_string());
        } else {
            return Err(format!("Unexpected compile argument: {arg}"));
        }
        i += 1;
    }

    if !create_executable
        && (!platforms.is_empty() || runtime_dir.is_some() || sea_mode || sea_config.is_some())
    {
        return Err(
            "compile: --platform, --runtime-dir, --sea, --sea-config, --asset, and --assets-dir require --executable".to_string(),
        );
    }

    if let Some(ref cfg_path) = sea_config {
        let cfg = load_sea_config(cfg_path)?;
        if input.is_none() {
            input = cfg.main;
        }
        if output.is_none() {
            output = cfg.output;
        }
    }

    let input = input.ok_or_else(|| "compile: input filename is required.".to_string())?;

    Ok(CompileArgs {
        input,
        output,
        outdir,
        platforms,
        create_executable,
        runtime_dir,
        sea_config,
        assets,
        assets_dir,
        sea_mode,
    })
}

struct ParsedSeaConfig {
    main: Option<PathBuf>,
    output: Option<String>,
    assets: Vec<(String, PathBuf)>,
}

fn resolve_sea_existing_path(cfg_dir: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() || p.exists() {
        return p;
    }
    let rel = cfg_dir.join(&p);
    if rel.exists() {
        return rel;
    }
    p
}

fn load_sea_config(path: &Path) -> Result<ParsedSeaConfig, String> {
    use simd_json::prelude::{ValueAsObject, ValueAsScalar, ValueTryIntoObject};

    let mut raw = fs::read(path)
        .map_err(|e| format!("Failed to read SEA config '{}': {e}", path.display()))?;
    let val = simd_json::to_owned_value(&mut raw)
        .map_err(|e| format!("Invalid JSON in SEA config '{}': {e}", path.display()))?;
    let obj = val
        .try_into_object()
        .map_err(|_| format!("SEA config '{}' must be a JSON object", path.display()))?;

    let cfg_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let main_raw = obj.get("main").and_then(|v| v.as_str().map(str::to_string));
    let main_rel_to_cfg = main_raw
        .as_deref()
        .map(|m| !Path::new(m).is_absolute() && !Path::new(m).exists() && cfg_dir.join(m).exists())
        .unwrap_or(false);

    let main = main_raw
        .as_deref()
        .map(|m| resolve_sea_existing_path(cfg_dir, m));

    let output = obj.get("output").and_then(|v| v.as_str()).map(|s| {
        let p = Path::new(s);
        if cfg_paths_should_join(p, main_rel_to_cfg) {
            cfg_dir.join(p).to_string_lossy().into_owned()
        } else {
            s.to_string()
        }
    });

    let mut assets = Vec::new();
    if let Some(assets_val) = obj.get("assets") {
        let assets_obj = assets_val.as_object().ok_or_else(|| {
            format!(
                "SEA config '{}' \"assets\" must be a JSON object",
                path.display()
            )
        })?;
        for (key, path_val) in assets_obj {
            let Some(asset_path_raw) = path_val.as_str() else {
                return Err(format!(
                    "SEA config '{}' asset '{key}' path must be a string",
                    path.display()
                ));
            };
            assets.push((
                key.clone(),
                resolve_sea_existing_path(cfg_dir, asset_path_raw),
            ));
        }
    }
    assets.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(ParsedSeaConfig {
        main,
        output,
        assets,
    })
}

fn cfg_paths_should_join(p: &Path, main_rel_to_cfg: bool) -> bool {
    !p.is_absolute() && main_rel_to_cfg
}

fn collect_dir_assets_recursive(
    base: &Path,
    cur: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let entries = fs::read_dir(cur)
        .map_err(|e| format!("Failed to read assets directory '{}': {e}", cur.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("Failed to read entry in '{}': {e}", cur.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_dir_assets_recursive(base, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| format!("Asset path error: {e}"))?;
            let key = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((key, path));
        }
    }
    Ok(())
}

fn collect_sea_asset_bytes(args: &CompileArgs) -> Result<Vec<(String, Vec<u8>)>, ExecError> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    let mut upsert = |key: String, path: PathBuf| {
        if let Some(existing) = entries.iter_mut().find(|(k, _)| *k == key) {
            existing.1 = path;
        } else {
            entries.push((key, path));
        }
    };

    if let Some(ref cfg_path) = args.sea_config {
        let cfg = load_sea_config(cfg_path)?;
        for (k, p) in cfg.assets {
            upsert(k, p);
        }
    }

    if let Some(ref dir) = args.assets_dir {
        let mut dir_entries = Vec::new();
        collect_dir_assets_recursive(dir, dir, &mut dir_entries)?;
        dir_entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, p) in dir_entries {
            upsert(k, p);
        }
    }

    for (k, p) in &args.assets {
        upsert(k.clone(), p.clone());
    }

    let mut out = Vec::with_capacity(entries.len());
    for (k, p) in entries {
        let bytes = fs::read(&p)
            .map_err(|e| format!("Failed to read SEA asset '{k}' from '{}': {e}", p.display()))?;
        out.push((k, bytes));
    }
    Ok(out)
}

fn split_flag(arg: &str) -> Option<(&str, Option<&str>)> {
    if !arg.starts_with('-') {
        return None;
    }
    if let Some((flag, val)) = arg.split_once('=') {
        Some((flag, Some(val)))
    } else {
        Some((arg, None))
    }
}

pub fn resolve_bytecode_output_path(args: &CompileArgs) -> PathBuf {
    if let Some(ref out) = args.output {
        if let Some(ref dir) = args.outdir {
            let p = Path::new(out);
            if p.components().count() == 1 {
                return dir.join(p);
            }
        }
        return PathBuf::from(out);
    }
    let mut buf = if let Some(ref dir) = args.outdir {
        let file_name = args
            .input
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("out"));
        dir.join(file_name)
    } else {
        args.input.clone()
    };
    buf.set_extension("lbt");
    buf
}

pub fn resolve_executable_targets(args: &CompileArgs) -> Vec<(Platform, PathBuf)> {
    let input_stem = args
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("app");

    if args.platforms.is_empty() {
        let platform = Platform::current();
        let path = match (&args.output, &args.outdir) {
            (Some(out), Some(dir)) => {
                let p = Path::new(out);
                if p.components().count() == 1 {
                    let mut out_path = dir.join(p);
                    if platform.is_windows() && out_path.extension().is_none() {
                        out_path.set_extension("exe");
                    }
                    out_path
                } else {
                    PathBuf::from(out)
                }
            },
            (Some(out), None) => PathBuf::from(out),
            (None, Some(dir)) => {
                let file_name = if platform.is_windows() {
                    format!("{input_stem}.exe")
                } else {
                    input_stem.to_string()
                };
                dir.join(file_name)
            },
            (None, None) => {
                let file_name = if platform.is_windows() {
                    format!("{input_stem}.exe")
                } else {
                    input_stem.to_string()
                };
                PathBuf::from(file_name)
            },
        };
        return vec![(platform, path)];
    }

    let outdir = args
        .outdir
        .clone()
        .unwrap_or_else(|| PathBuf::from("./dist"));
    let base_name = args
        .output
        .as_deref()
        .map(|raw| {
            Path::new(raw)
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(raw)
        })
        .unwrap_or(input_stem);

    args.platforms
        .iter()
        .map(|&platform| {
            let file_name = if platform.is_windows() {
                format!("{base_name}-{}.exe", platform.as_str())
            } else {
                format!("{base_name}-{}", platform.as_str())
            };
            (platform, outdir.join(file_name))
        })
        .collect()
}

pub fn extract_embedded_bytecode() -> Option<Vec<u8>> {
    match libsui::find_section(SECTION_NAME) {
        Ok(Some(data)) => Some(data.to_vec()),
        _ => None,
    }
}

pub fn inject_bytecode(
    platform: Platform,
    runtime_binary: &[u8],
    bytecode: &[u8],
    output_path: &Path,
) -> Result<(), ExecError> {
    if let Some(parent) = output_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    if output_path.exists() {
        fs::remove_file(output_path)?;
    }

    let mut output = File::create(output_path)?;
    match platform {
        Platform::LinuxX64Gnu
        | Platform::LinuxArm64Gnu
        | Platform::LinuxX64Musl
        | Platform::LinuxArm64Musl => {
            libsui::Elf::new(runtime_binary).append(SECTION_NAME, bytecode, &mut output)?;
        },
        Platform::DarwinX64 | Platform::DarwinArm64 => {
            libsui::Macho::from(runtime_binary.to_vec())?
                .write_section(SECTION_NAME, bytecode.to_vec())?
                .build_and_sign(&mut output)?;
        },
        Platform::WindowsX64 | Platform::WindowsArm64 => {
            libsui::PortableExecutable::from(runtime_binary)?
                .write_resource(SECTION_NAME, bytecode.to_vec())?
                .build(&mut output)?;
        },
    }
    drop(output);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(output_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(output_path, perms)?;
    }

    Ok(())
}

pub fn default_cache_root() -> Option<PathBuf> {
    if let Ok(custom) = env::var("LEXE_CACHE_DIR") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if cfg!(windows) {
        env::var("LOCALAPPDATA")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|dir| PathBuf::from(dir).join("lexe").join("cache"))
    } else if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("lexe"));
        }
        env::var("HOME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|home| PathBuf::from(home).join(".cache").join("lexe"))
    } else {
        env::var("HOME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|home| PathBuf::from(home).join(".cache").join("lexe"))
    }
}

pub fn version_cache_dir() -> Option<PathBuf> {
    default_cache_root().map(|root| root.join(format!("v{VERSION}")))
}

fn candidate_binary_paths(dir: &Path, platform: Platform) -> [PathBuf; 4] {
    let exe_name = if platform.is_windows() {
        "lexe.exe"
    } else {
        "lexe"
    };
    let platform_bin = if platform.is_windows() {
        format!("lexe-{}.exe", platform.as_str())
    } else {
        format!("lexe-{}", platform.as_str())
    };
    [
        dir.join(platform.release_asset_name()),
        dir.join(platform_bin),
        dir.join(format!("lexe-{}", platform.as_str()))
            .join(exe_name),
        dir.join(platform.as_str()).join(exe_name),
    ]
}

pub fn find_cached_runtime_binary(
    platform: Platform,
    runtime_dir: Option<&Path>,
) -> Option<PathBuf> {
    let mut search_dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = runtime_dir {
        search_dirs.push(dir.to_path_buf());
    }
    if let Ok(env_dir) = env::var("LEXE_BIN_DIR") {
        if !env_dir.trim().is_empty() {
            search_dirs.push(PathBuf::from(env_dir));
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            search_dirs.push(parent.to_path_buf());
            if let Some(grandparent) = parent.parent() {
                search_dirs.push(grandparent.to_path_buf());
            }
        }
    }
    if let Some(cache_dir) = version_cache_dir() {
        search_dirs.push(cache_dir);
    }

    for dir in search_dirs {
        for candidate in candidate_binary_paths(&dir, platform) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn release_base_url() -> String {
    if let Ok(custom) = env::var("LEXE_RELEASE_BASE_URL") {
        let trimmed = custom.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    format!("{DEFAULT_RELEASE_BASE_URL}/v{VERSION}")
}

pub fn parse_sha256sums_entry(contents: &str, asset_name: &str) -> Result<String, String> {
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(digest) = parts.next() else {
            continue;
        };
        let Some(raw_file) = parts.next() else {
            continue;
        };
        let file = raw_file.trim_start_matches('*');
        if file == asset_name {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!(
                    "Invalid SHA256 digest '{digest}' for asset '{asset_name}' in SHA256SUMS"
                ));
            }
            return Ok(digest.to_ascii_lowercase());
        }
    }
    Err(format!("Asset '{asset_name}' not found in SHA256SUMS"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(all(
    any(feature = "base", feature = "http"),
    any(
        feature = "tls-ring",
        feature = "tls-aws-lc",
        feature = "tls-graviola",
        feature = "tls-openssl"
    )
))]
async fn send_get(uri: &hyper::Uri) -> Result<hyper::Response<hyper::body::Incoming>, ExecError> {
    use bytes::Bytes;
    use http_body_util::{BodyExt as _, Empty};
    use hyper::{header, Request};

    let client = lexe_http::HTTP_CLIENT
        .as_ref()
        .map_err(|e| format!("Failed to initialize HTTP client for runtime download: {e}"))?;

    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::USER_AGENT, format!("lexe/{VERSION}"))
        .header(header::ACCEPT, "*/*")
        .body(
            Empty::<Bytes>::new()
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        )?;

    Ok(client.request(req).await?)
}

#[cfg(all(
    any(feature = "base", feature = "http"),
    any(
        feature = "tls-ring",
        feature = "tls-aws-lc",
        feature = "tls-graviola",
        feature = "tls-openssl"
    )
))]
pub(crate) async fn fetch_redirect_location(url: &str) -> Result<String, ExecError> {
    use hyper::{header, StatusCode, Uri};

    let uri: Uri = url
        .parse()
        .map_err(|e| format!("Invalid URL '{url}': {e}"))?;
    let resp = send_get(&uri).await?;
    let status = resp.status();

    if !matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    ) {
        return Err(format!("Expected redirect from {url}, got HTTP {status}").into());
    }

    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| format!("Redirect ({status}) missing Location header for {url}"))?;

    resolve_redirect_url(&uri, location)
}

#[cfg(all(
    any(feature = "base", feature = "http"),
    any(
        feature = "tls-ring",
        feature = "tls-aws-lc",
        feature = "tls-graviola",
        feature = "tls-openssl"
    )
))]
pub(crate) async fn fetch_url_bytes(url: &str) -> Result<Vec<u8>, ExecError> {
    use http_body_util::BodyExt;
    use hyper::{header, StatusCode, Uri};

    const MAX_REDIRECTS: usize = 10;

    let mut current_url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let uri: Uri = current_url
            .parse()
            .map_err(|e| format!("Invalid URL '{current_url}': {e}"))?;

        let resp = send_get(&uri).await?;
        let status = resp.status();

        if matches!(
            status,
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            let location = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    format!("Redirect ({status}) missing Location header for {current_url}")
                })?;

            current_url = resolve_redirect_url(&uri, location)?;
            continue;
        }

        if !status.is_success() {
            return Err(format!("HTTP {status} when downloading {current_url}").into());
        }

        let collected = resp.into_body().collect().await?;
        return Ok(collected.to_bytes().to_vec());
    }

    Err(format!("Too many redirects while downloading {url}").into())
}

#[cfg(all(
    any(feature = "base", feature = "http"),
    any(
        feature = "tls-ring",
        feature = "tls-aws-lc",
        feature = "tls-graviola",
        feature = "tls-openssl"
    )
))]
fn resolve_redirect_url(base: &hyper::Uri, location: &str) -> Result<String, ExecError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }
    let scheme = base
        .scheme_str()
        .ok_or_else(|| format!("Missing URL scheme in '{base}'"))?;
    let authority = base
        .authority()
        .ok_or_else(|| format!("Missing URL authority in '{base}'"))?;
    if location.starts_with("//") {
        return Ok(format!("{scheme}:{location}"));
    }
    if location.starts_with('/') {
        return Ok(format!("{scheme}://{authority}{location}"));
    }
    let base_path = base.path();
    let dir = match base_path.rfind('/') {
        Some(idx) => &base_path[..=idx],
        None => "/",
    };
    Ok(format!("{scheme}://{authority}{dir}{location}"))
}

#[cfg(not(all(
    any(feature = "base", feature = "http"),
    any(
        feature = "tls-ring",
        feature = "tls-aws-lc",
        feature = "tls-graviola",
        feature = "tls-openssl"
    )
)))]
pub(crate) async fn fetch_redirect_location(url: &str) -> Result<String, ExecError> {
    Err(format!("Cannot query {url}: this lexe binary was built without HTTP/TLS support.").into())
}

#[cfg(not(all(
    any(feature = "base", feature = "http"),
    any(
        feature = "tls-ring",
        feature = "tls-aws-lc",
        feature = "tls-graviola",
        feature = "tls-openssl"
    )
)))]
pub(crate) async fn fetch_url_bytes(url: &str) -> Result<Vec<u8>, ExecError> {
    Err(format!(
        "Cannot download {url}: this lexe binary was built without HTTP/TLS support. Provide prebuilt binaries via --runtime-dir or LEXE_BIN_DIR."
    )
    .into())
}

async fn download_and_cache_runtime_binary(
    platform: Platform,
    sha256sums_cache: &mut Option<String>,
) -> Result<Vec<u8>, ExecError> {
    let base_url = release_base_url();
    if sha256sums_cache.is_none() {
        let sums_url = format!("{base_url}/SHA256SUMS");
        eprintln!("Fetching checksums from {sums_url}...");
        let sums_bytes = fetch_url_bytes(&sums_url).await?;
        let sums_text = String::from_utf8(sums_bytes)
            .map_err(|e| format!("Invalid UTF-8 in SHA256SUMS from {sums_url}: {e}"))?;
        *sha256sums_cache = Some(sums_text);
    }

    let asset = platform.release_asset_name();
    let expected_sha = parse_sha256sums_entry(sha256sums_cache.as_deref().unwrap(), asset)
        .map_err(|e| format!("Checksum lookup failed for {asset}: {e}"))?;

    let asset_url = format!("{base_url}/{asset}");
    eprintln!("Downloading {asset} from {asset_url}...");
    let bytes = fetch_url_bytes(&asset_url).await?;

    let actual_sha = sha256_hex(&bytes);
    if actual_sha != expected_sha {
        return Err(format!(
            "SHA256 verification failed for {asset}: expected {expected_sha}, got {actual_sha}"
        )
        .into());
    }
    eprintln!("Verified SHA256 for {asset}");

    if let Some(cache_dir) = version_cache_dir() {
        fs::create_dir_all(&cache_dir)?;
        let tmp_name = format!(".{asset}.{}.tmp", std::process::id());
        let tmp_path = cache_dir.join(tmp_name);
        let final_path = cache_dir.join(asset);

        if let Err(err) = write_and_promote_temp(&tmp_path, &final_path, &bytes) {
            let _ = fs::remove_file(&tmp_path);
            return Err(err);
        }
    }

    Ok(bytes)
}

fn write_and_promote_temp(
    tmp_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<(), ExecError> {
    fs::write(tmp_path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(tmp_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(tmp_path, perms)?;
    }
    #[cfg(windows)]
    if final_path.exists() {
        let _ = fs::remove_file(final_path);
    }
    fs::rename(tmp_path, final_path)?;
    Ok(())
}

async fn resolve_runtime_binary(
    platform: Platform,
    runtime_dir: Option<&Path>,
    sha256sums_cache: &mut Option<String>,
) -> Result<Vec<u8>, ExecError> {
    let has_explicit_dir = runtime_dir.is_some()
        || env::var("LEXE_BIN_DIR")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);

    if platform == Platform::current() && !has_explicit_dir {
        if extract_embedded_bytecode().is_some() {
            return Err(
                "Current lexe binary already contains an embedded executable payload; provide a clean runtime binary via --runtime-dir or LEXE_BIN_DIR".into(),
            );
        }
        let exe_path = env::current_exe()?;
        return Ok(fs::read(exe_path)?);
    }

    if let Some(found) = find_cached_runtime_binary(platform, runtime_dir) {
        return Ok(fs::read(found)?);
    }

    download_and_cache_runtime_binary(platform, sha256sums_cache).await
}

pub async fn run_compile_executable(args: &CompileArgs) -> Result<(), ExecError> {
    let compiled = compile_file_to_bytes(&args.input).await?;
    let payload = if args.sea_mode {
        let assets = collect_sea_asset_bytes(args)?;
        lexe_sea::container::encode_sea_payload(&compiled, &assets, true)?
    } else {
        compiled
    };
    let targets = resolve_executable_targets(args);
    let mut sha256sums_cache: Option<String> = None;

    for (platform, output_path) in targets {
        let runtime_binary =
            resolve_runtime_binary(platform, args.runtime_dir.as_deref(), &mut sha256sums_cache)
                .await?;

        inject_bytecode(platform, &runtime_binary, &payload, &output_path)?;
        println!(
            "Created executable ({}) -> {}",
            platform.as_str(),
            output_path.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_linux_libc_variants_have_distinct_assets() {
        assert_eq!(
            Platform::LinuxX64Gnu.release_asset_name(),
            "lexe-linux-x64-gnu"
        );
        assert_eq!(
            Platform::LinuxArm64Gnu.release_asset_name(),
            "lexe-linux-arm64-gnu"
        );
        assert_eq!(
            Platform::LinuxX64Musl.release_asset_name(),
            "lexe-linux-x64-musl"
        );
        assert_eq!(
            Platform::LinuxArm64Musl.release_asset_name(),
            "lexe-linux-arm64-musl"
        );
        // Every Linux asset must be unique and follow the libc suffix convention.
        let mut names: Vec<_> = Platform::all()
            .iter()
            .map(|p| p.release_asset_name())
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Platform::all().len());
    }

    #[test]
    fn test_platform_parsing_and_aliases() {
        assert_eq!(
            Platform::from_str("linux-x64").unwrap(),
            Platform::LinuxX64Gnu
        );
        assert_eq!(
            Platform::from_str("linux-arm64").unwrap(),
            Platform::LinuxArm64Gnu
        );
        assert_eq!(
            Platform::from_str("darwin-x64").unwrap(),
            Platform::DarwinX64
        );
        assert_eq!(
            Platform::from_str("macos-x64").unwrap(),
            Platform::DarwinX64
        );
        assert_eq!(
            Platform::from_str("darwin-arm64").unwrap(),
            Platform::DarwinArm64
        );
        assert_eq!(
            Platform::from_str("macos-arm64").unwrap(),
            Platform::DarwinArm64
        );
        assert_eq!(
            Platform::from_str("windows-x64").unwrap(),
            Platform::WindowsX64
        );
        assert_eq!(
            Platform::from_str("windows-arm64").unwrap(),
            Platform::WindowsArm64
        );
        assert_eq!(
            Platform::from_str("linux-x64-musl").unwrap(),
            Platform::LinuxX64Musl
        );
        assert_eq!(
            Platform::from_str("linux-arm64-musl").unwrap(),
            Platform::LinuxArm64Musl
        );
        assert_eq!(
            Platform::from_str("linux-x64-gnu").unwrap(),
            Platform::LinuxX64Gnu
        );
        assert!(Platform::from_str("freebsd-x64").is_err());
    }

    #[test]
    fn test_parse_compile_args_variants() {
        let single =
            parse_compile_args(&["app.js".into(), "my-app".into(), "--executable".into()]).unwrap();
        assert_eq!(single.input, PathBuf::from("app.js"));
        assert_eq!(single.output.as_deref(), Some("my-app"));
        assert!(single.create_executable);
        assert!(single.platforms.is_empty());

        let targets = resolve_executable_targets(&single);
        assert_eq!(
            targets,
            vec![(Platform::current(), PathBuf::from("my-app"))]
        );

        let multi = parse_compile_args(&[
            "app.js".into(),
            "--executable".into(),
            "-o=cli".into(),
            "-d".into(),
            "out".into(),
            "-p=linux-x64,windows-arm64,macos-arm64".into(),
            "--runtime-dir=/tmp/bins".into(),
        ])
        .unwrap();
        assert_eq!(
            multi.platforms,
            vec![
                Platform::LinuxX64Gnu,
                Platform::WindowsArm64,
                Platform::DarwinArm64,
            ]
        );
        assert_eq!(multi.runtime_dir, Some(PathBuf::from("/tmp/bins")));

        let multi_targets = resolve_executable_targets(&multi);
        assert_eq!(
            multi_targets,
            vec![
                (Platform::LinuxX64Gnu, PathBuf::from("out/cli-linux-x64")),
                (
                    Platform::WindowsArm64,
                    PathBuf::from("out/cli-windows-arm64.exe"),
                ),
                (Platform::DarwinArm64, PathBuf::from("out/cli-darwin-arm64")),
            ]
        );

        assert!(parse_compile_args(&[]).is_err());
        assert!(parse_compile_args(&["app.js".into(), "-p".into(), "linux-x64".into()]).is_err());
        assert!(parse_compile_args(&["app.js".into(), "--sea".into()]).is_err());
        assert!(parse_compile_args(&["app.js".into(), "--unknown".into()]).is_err());
    }

    #[test]
    fn test_parse_sea_compile_args_and_config() {
        let _guard = env_lock().blocking_lock();
        let root = temp_test_dir("lexe-sea-cfg-parse");
        let cfg_path = root.join("sea.json");
        let entry_path = root.join("main.mjs");
        let asset_path = root.join("hello.txt");
        fs::write(&entry_path, b"console.log('hi');").unwrap();
        fs::write(&asset_path, b"hello asset").unwrap();
        fs::write(
            &cfg_path,
            r#"{
              "main": "main.mjs",
              "mainFormat": "module",
              "output": "out/my-sea",
              "disableExperimentalSEAWarning": true,
              "assets": { "greeting.txt": "hello.txt" }
            }"#,
        )
        .unwrap();

        let parsed = parse_compile_args(&[
            "--executable".into(),
            "--sea-config".into(),
            cfg_path.to_string_lossy().into_owned(),
            "--asset".into(),
            format!("extra={}", asset_path.display()),
        ])
        .unwrap();
        assert!(parsed.sea_mode);
        assert_eq!(parsed.input, entry_path);
        assert_eq!(
            parsed.output.as_deref(),
            Some(root.join("out/my-sea").to_string_lossy().as_ref())
        );

        let collected = collect_sea_asset_bytes(&parsed).unwrap();
        assert_eq!(
            collected,
            vec![
                ("greeting.txt".to_string(), b"hello asset".to_vec()),
                ("extra".to_string(), b"hello asset".to_vec()),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_parse_sha256sums_entry() {
        let valid_hex = "a".repeat(64);
        let other_hex = "b".repeat(64);
        let sums =
            format!("{other_hex}  lexe-linux-x64-gnu\n{valid_hex} *lexe-windows-arm64.exe\n");

        assert_eq!(
            parse_sha256sums_entry(&sums, "lexe-linux-x64-gnu").unwrap(),
            other_hex
        );
        assert_eq!(
            parse_sha256sums_entry(&sums, "lexe-windows-arm64.exe").unwrap(),
            valid_hex
        );
        assert!(parse_sha256sums_entry(&sums, "lexe-macos-x64").is_err());

        let bad_sums = "nothex  lexe-linux-x64-gnu\n";
        assert!(parse_sha256sums_entry(bad_sums, "lexe-linux-x64-gnu").is_err());
    }

    #[test]
    fn test_find_cached_runtime_binary_precedence() {
        let _guard = env_lock().blocking_lock();
        let root = temp_test_dir("lexe-cache-precedence");
        let dir_a = root.join("dir_a");
        let dir_b = root.join("dir_b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        let bin_a = dir_a.join("lexe-linux-x64-gnu");
        let bin_b = dir_b.join("lexe-linux-x64-gnu");
        fs::write(&bin_a, b"from-a").unwrap();
        fs::write(&bin_b, b"from-b").unwrap();

        env::set_var("LEXE_BIN_DIR", &dir_b);
        let found = find_cached_runtime_binary(Platform::LinuxX64Gnu, Some(&dir_a));
        env::remove_var("LEXE_BIN_DIR");

        assert_eq!(found.as_deref(), Some(bin_a.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(all(
        any(feature = "base", feature = "http"),
        any(
            feature = "tls-ring",
            feature = "tls-aws-lc",
            feature = "tls-graviola",
            feature = "tls-openssl"
        )
    ))]
    #[tokio::test]
    async fn test_auto_download_redirect_and_sha256_verification() {
        let _guard = env_lock().lock().await;

        let cache_dir = temp_test_dir("lexe-dl-cache");
        // Pick a Linux target that is not the host, so resolve_runtime_binary takes the
        // download path instead of returning the running test binary.
        let target = if Platform::current() == Platform::LinuxArm64Gnu {
            Platform::LinuxX64Gnu
        } else {
            Platform::LinuxArm64Gnu
        };
        let payload = b"FAKE_LINUX_BINARY_PAYLOAD".to_vec();
        let digest = sha256_hex(&payload);
        let asset = target.release_asset_name();
        let sha256sums = format!("{digest}  {asset}\n");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_payload = payload.clone();
        let server_sums = sha256sums.clone();
        let server = tokio::spawn(async move {
            // Expect 3 requests: GET /SHA256SUMS, GET /<asset> (302), GET /cdn/<asset>
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let path = first_line.split_whitespace().nth(1).unwrap_or("/");

                if path == "/SHA256SUMS" {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        server_sums.len(),
                        server_sums
                    );
                    socket.write_all(resp.as_bytes()).await.unwrap();
                } else if path == format!("/{asset}") {
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: /cdn/{asset}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    socket.write_all(resp.as_bytes()).await.unwrap();
                } else if path == format!("/cdn/{asset}") {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        server_payload.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(&server_payload).await.unwrap();
                } else {
                    socket
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                }
            }
        });

        env::set_var("LEXE_CACHE_DIR", &cache_dir);
        env::set_var("LEXE_RELEASE_BASE_URL", format!("http://{addr}"));
        env::remove_var("LEXE_BIN_DIR");

        let mut sums_cache = None;
        let downloaded = resolve_runtime_binary(target, None, &mut sums_cache)
            .await
            .unwrap();
        assert_eq!(downloaded, payload);

        let cached_file = cache_dir.join(format!("v{VERSION}")).join(asset);
        assert!(cached_file.is_file());
        assert_eq!(fs::read(&cached_file).unwrap(), payload);

        server.await.unwrap();

        // Second call should hit cache even after server shut down.
        let from_cache = resolve_runtime_binary(target, None, &mut sums_cache)
            .await
            .unwrap();
        assert_eq!(from_cache, payload);

        env::remove_var("LEXE_CACHE_DIR");
        env::remove_var("LEXE_RELEASE_BASE_URL");
        let _ = fs::remove_dir_all(&cache_dir);
    }

    #[cfg(all(
        any(feature = "base", feature = "http"),
        any(
            feature = "tls-ring",
            feature = "tls-aws-lc",
            feature = "tls-graviola",
            feature = "tls-openssl"
        )
    ))]
    #[tokio::test]
    async fn test_auto_download_rejects_sha256_mismatch() {
        let _guard = env_lock().lock().await;

        let cache_dir = temp_test_dir("lexe-dl-bad-sha");
        let asset = Platform::LinuxArm64Gnu.release_asset_name();
        let bad_sums = format!("{}  {asset}\n", "0".repeat(64));
        let payload = b"CORRUPTED_OR_TAMPERED_BYTES".to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let path = first_line.split_whitespace().nth(1).unwrap_or("/");

                if path == "/SHA256SUMS" {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        bad_sums.len(),
                        bad_sums
                    );
                    socket.write_all(resp.as_bytes()).await.unwrap();
                } else {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(&payload).await.unwrap();
                }
            }
        });

        env::set_var("LEXE_CACHE_DIR", &cache_dir);
        env::set_var("LEXE_RELEASE_BASE_URL", format!("http://{addr}"));
        env::remove_var("LEXE_BIN_DIR");

        let mut sums_cache = None;
        let err = resolve_runtime_binary(Platform::LinuxArm64Gnu, None, &mut sums_cache)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("SHA256 verification failed"),
            "unexpected error: {err}"
        );

        let cached_file = cache_dir.join(format!("v{VERSION}")).join(asset);
        assert!(!cached_file.exists());

        server.await.unwrap();
        env::remove_var("LEXE_CACHE_DIR");
        env::remove_var("LEXE_RELEASE_BASE_URL");
        let _ = fs::remove_dir_all(&cache_dir);
    }
}
