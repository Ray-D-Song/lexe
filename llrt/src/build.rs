use libsui::{find_section, Elf, Macho, PortableExecutable};
use llrt_core::compiler::compile_string;
use pico_args::Arguments;
use std::env;
use std::fs::File;
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::str::FromStr;

static MAGIC_NUMBER: &str = "1exe6und1e";
static SECTION_NAME: &str = "1exec0de";

/**
 * Build::validate_args(args)
 *     .run_build()
 */
#[derive(Debug, Clone, PartialEq)]
enum Platform {
    LinuxX64,
    LinuxArm64,
    DarwinX64,
    DarwinArm64,
    WindowsX64,
    WindowsArm64,
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "linux-x64" => Ok(Platform::LinuxX64),
            "linux-arm64" => Ok(Platform::LinuxArm64),
            "darwin-x64" => Ok(Platform::DarwinX64),
            "darwin-arm64" => Ok(Platform::DarwinArm64),
            "windows-x64" => Ok(Platform::WindowsX64),
            "windows-arm64" => Ok(Platform::WindowsArm64),
            _ => Err(format!("Unknown platform: {}", s)),
        }
    }
}

#[derive(Debug)]
struct BuildArgs {
    /// Input file (required)
    input: PathBuf,

    /// Output file name (optional, default: input-<platform>)
    output: Option<String>,

    /// Output directory (optional, default: ./dist)
    directory: PathBuf,

    /// Target platform(s), comma-separated
    platform: Vec<Platform>,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "linux") {
            if cfg!(target_arch = "x86_64") {
                Platform::LinuxX64
            } else {
                Platform::LinuxArm64
            }
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "x86_64") {
                Platform::DarwinX64
            } else {
                Platform::DarwinArm64
            }
        } else {
            if cfg!(target_arch = "x86_64") {
                Platform::WindowsX64
            } else {
                Platform::WindowsArm64
            }
        }
    }
}

pub struct LexeBuild {
    args: BuildArgs,
}

type BuildErr = Box<dyn std::error::Error + Send + Sync>;

impl LexeBuild {
    /// Validate build arguments and return a Build instance
    pub fn validate_args() -> Result<Self, String> {
        let mut pargs = Arguments::from_env();

        println!("{:?}", pargs);

        // ignore the first argument (build)
        let _ = pargs.opt_value_from_str::<_, String>("build").ok();

        // Parse input (required) - try both flag and positional formats
        let input = match pargs
            .value_from_str::<_, PathBuf>("--input")
            .or_else(|_| pargs.value_from_str::<_, PathBuf>("-i"))
        {
            Ok(path) => path,
            Err(_) => {
                // Try as positional argument if flag parsing failed
                if let Some(arg) = pargs.free_from_str::<String>().ok() {
                    PathBuf::from(arg)
                } else {
                    return Err("Input file is required. Use --input <file> or -i <file> or provide as positional argument".to_string());
                }
            },
        };

        // Parse output (optional)
        let output = match pargs
            .opt_value_from_str::<_, String>("--output")
            .or_else(|_| pargs.opt_value_from_str::<_, String>("-o"))
        {
            Ok(val) => val,
            Err(e) => return Err(format!("Failed to parse output name: {}", e)),
        };

        // Parse directory (optional, default: "dist")
        let directory = match pargs
            .opt_value_from_str::<_, PathBuf>("--directory")
            .or_else(|_| pargs.opt_value_from_str::<_, PathBuf>("-d"))
        {
            Ok(Some(dir)) => dir,
            Ok(None) => PathBuf::from("dist"),
            Err(e) => return Err(format!("Failed to parse directory path: {}", e)),
        };

        // Parse platforms (optional, default: current platform)
        let platform_str = match pargs
            .opt_value_from_str::<_, String>("--platform")
            .or_else(|_| pargs.opt_value_from_str::<_, String>("-p"))
        {
            Ok(Some(p)) => p,
            Ok(None) => platform_to_str(&Platform::current()).to_string(),
            Err(e) => return Err(format!("Failed to parse platform: {}", e)),
        };

        let platform = platform_str
            .split(',')
            .map(|s| Platform::from_str(s.trim()))
            .collect::<Result<Vec<_>, _>>()?;

        if platform.is_empty() {
            return Err("At least one platform must be specified".to_string());
        }

        // Check for unused arguments
        let remaining = pargs.finish();
        if !remaining.is_empty() {
            return Err(format!("Unknown arguments: {:?}", remaining));
        }

        let args = BuildArgs {
            input,
            output,
            directory,
            platform,
        };

        // Ensure input file exists
        if !args.input.exists() {
            return Err(format!("Input file not found: {}", args.input.display()));
        }

        // Create output directory if it doesn't exist
        if !args.directory.exists() {
            if let Err(e) = std::fs::create_dir_all(&args.directory) {
                return Err(format!("Failed to create output directory: {}", e));
            }
        }

        Ok(LexeBuild { args })
    }

    /// Execute the build process
    pub async fn run_build(self) -> Result<(), BuildErr> {
        println!("Starting build process...");
        println!("Compile input file: {}", self.args.input.display());

        // read input file
        let input_str = match std::fs::read_to_string(&self.args.input) {
            Ok(content) => content,
            Err(e) => return Err(format!("Failed to read input file: {}", e).into()),
        };
        // compile input file
        let compiled = match compile_string(&input_str).await {
            Ok(bytes) => bytes,
            Err(e) => return Err(format!("Failed to compile input file: {}", e).into()),
        };

        // get current executable path
        // use this path to find other platform's llrt binary
        let current_exe_path = env::current_exe()?;
        let current_exe_dir = current_exe_path
            .parent()
            .ok_or("Failed to get current executable directory")?;
        let root_dir = current_exe_dir
            .parent()
            .ok_or("Failed to get root directory")?;

        for platform in &self.args.platform {
            let output_name = self.output_name_for_platform(platform);
            let output_path = self.args.directory.join(output_name);

            println!("Building for {:?} -> {}", platform, output_path.display());

            let cache_path = root_dir
                .join(format!("llrt-{}", platform_to_str(platform)))
                .join("llrt");

            if !cache_path.exists() {
                return Err(format!("Cache path does not exist: {}", cache_path.display()).into());
            }
            // read llrt binary file
            let llrt_binary = match std::fs::read(&cache_path) {
                Ok(bytes) => bytes,
                Err(e) => return Err(format!("Failed to read llrt binary: {}", e).into()),
            };

            if output_path.exists() {
                // delete existing output file
                if let Err(e) = std::fs::remove_file(&output_path) {
                    return Err(format!("Failed to remove existing output file: {}", e).into());
                }
            }
            let mut output = File::create(output_path)?;
            if platform == &Platform::WindowsX64 || platform == &Platform::WindowsArm64 {
                PortableExecutable::from(&llrt_binary)?
                    .write_resource(SECTION_NAME, compiled.clone())?
                    .build(&mut output)?;
            } else if platform == &Platform::LinuxX64 || platform == &Platform::LinuxArm64 {
                let elf = Elf::new(&llrt_binary);
                elf.append(SECTION_NAME, &compiled, &mut output)?;
            } else if platform == &Platform::DarwinX64 || platform == &Platform::DarwinArm64 {
                Macho::from(llrt_binary)?
                    .write_section(SECTION_NAME, compiled.clone())?
                    .build_and_sign(&mut output)?;
            }

            // append MAGIC_NUMBER
            output.write_all(MAGIC_NUMBER.as_bytes())?;
        }

        println!("Build completed successfully");
        Ok(())
    }

    fn output_name_for_platform(&self, platform: &Platform) -> String {
        self.args.output.as_ref().map_or_else(
            || {
                let stem = self
                    .args
                    .input
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("output");
                format!("{}-{}", stem, platform_to_str(platform))
            },
            |name| name.clone(),
        )
    }
}

fn platform_to_str(platform: &Platform) -> &str {
    match platform {
        Platform::LinuxX64 => "linux-x64",
        Platform::LinuxArm64 => "linux-arm64",
        Platform::DarwinX64 => "darwin-x64",
        Platform::DarwinArm64 => "darwin-arm64",
        Platform::WindowsX64 => "windows-x64",
        Platform::WindowsArm64 => "windows-arm64",
    }
}

pub fn has_magic_number() -> std::io::Result<bool> {
    let path = env::current_exe()?;
    let mut file = File::open(path)?;

    let file_size = file.metadata()?.len() as usize;
    if file_size < MAGIC_NUMBER.len() {
        return Ok(false); // file is too small to contain magic number
    }

    let search_area_size = 1024.min(file_size);
    let search_start = file_size.saturating_sub(search_area_size);

    file.seek(SeekFrom::Start(search_start as u64))?;

    let mut buffer = vec![0; search_area_size];
    file.read_exact(&mut buffer)?;

    // search from end to start
    for i in (0..search_area_size - MAGIC_NUMBER.len() + 1).rev() {
        if &buffer[i..i + MAGIC_NUMBER.len()] == MAGIC_NUMBER.as_bytes() {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn extract_code_binary() -> Option<Vec<u8>> {
    let Some(data) = find_section(SECTION_NAME) else {
        eprintln!("Error finding section");
        return None;
    };

    Some(data.to_vec())
}
