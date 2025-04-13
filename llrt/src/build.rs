use libsui::{find_section, Elf, Macho, PortableExecutable};
use llrt_core::compiler::compile_string;
use std::fs::File;
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::str::FromStr;
use std::env;

static MAGIC_NUMBER: &str = "1exe6und1e";
static LIBSUI_MAGIC_NUMBER: u32 = 0x501e;
static SECTION_NAME: &str = "1exec0de";

/**
 * Build::validate_args(args)
 *     .run_build()
 */
#[derive(Debug, Clone, PartialEq)]
pub enum Platform {
    LinuxX64,
    LinuxArm64,
    DarwinX64,
    DarwinArm64,
    WindowsX64,
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
            _ => Err(format!("Unknown platform: {}", s)),
        }
    }
}

#[derive(Debug)]
struct BuildArgs {
    /// Input file (required)
    input: PathBuf,

    /// Output file name (optional, default: input file name)
    output: String,

    /// Output directory (optional, default: ./dist)
    directory: PathBuf,

    /// Target platform(s), comma-separated
    platform: Vec<Platform>,
}

impl Platform {
    pub fn current() -> Self {
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
            Platform::WindowsX64
        }
    }
}

pub struct LexeBuild {
    args: BuildArgs,
}

type BuildErr = Box<dyn std::error::Error + Send + Sync>;

impl LexeBuild {
    /// Validate build arguments and return a Build instance
    pub fn validate_args(args: &[String]) -> Result<Self, String> {
        let mut input: Option<PathBuf> = None;
        let mut output = String::new();
        let mut directory: Option<PathBuf> = None;
        let mut platform = Vec::new();

        for arg in args.iter().filter(|arg| arg.contains('=')) {
            let parts: Vec<&str> = arg.split('=').collect();
            
            // check argument format
            if parts.len() != 2 {
                return Err(format!("Invalid argument format: {}", arg));
            }

            match parts[0] {
                "-i" => input = Some(PathBuf::from(parts[1])),
                "-o" => output = parts[1].to_string(),
                "-d" => directory = Some(PathBuf::from(parts[1])),
                "-p" => {
                    let parsed_platforms: Result<Vec<Platform>, _> = 
                        parts[1].split(',')
                        .map(|p| p.parse::<Platform>())
                        .collect();
                    
                    match parsed_platforms {
                        Ok(platforms) => platform = platforms,
                        Err(e) => return Err(format!("Invalid platform: {}", e)),
                    }
                },
                _ => return Err(format!("Unknown argument: {}", parts[0])),
            }
        }

        // validate input
        if input.is_none() {
            return Err("Input file is required".to_string());
        }

        // default platform
        if platform.is_empty() {
            platform.push(Platform::current());
        }

        // default output
        if output.is_empty () {
            // get input file name and remove extension
            let input_file_name = input.as_ref().unwrap().file_name().unwrap().to_str().unwrap();
            let input_file_name = input_file_name.split('.').next().unwrap();
            output = input_file_name.to_string();
        }

        // default directory
        if directory.is_none() {
            directory = Some(PathBuf::from("./dist"));
        }

        let args = BuildArgs { input: input.unwrap(), output, directory: directory.unwrap(), platform };
        Ok(LexeBuild { args })
    }

    /// Execute the build process
    pub async fn run_build(self) -> Result<(), BuildErr> {
        println!("\x1b[33mCompile input file: {}\x1b[0m", self.args.input.display());

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
            let output_path = self.output_path_for_platform(platform);

            // Print the build platform and output path in yellow
            println!("\x1b[33mBuilding for: {:?} -> {}\x1b[0m", platform, output_path.display());

            let cache_path = root_dir
                .join(format!("llrt-{}", platform_to_str(platform)))
                .join(platform_to_binary_name(platform));

            // check if the file in cache_path exists
            if !cache_path.exists() {
                return Err(format!("LLRT binary not found in cache: {}", cache_path.display()).into());
            }

            // read llrt binary file
            let llrt_binary = match std::fs::read(&cache_path) {
                Ok(bytes) => bytes,
                Err(e) => return Err(format!("Failed to read llrt binary: {}", e).into()),
            };

            // Ensure the output directory exists before creating the file
            if let Some(parent_dir) = output_path.parent() {
                std::fs::create_dir_all(parent_dir)?;
            } else {
                // This case should ideally not happen with the current logic,
                // but it's good practice to handle it.
                return Err(format!("Could not determine parent directory for output path: {}", output_path.display()).into());
            }

            if output_path.exists() {
                // delete existing output file
                if let Err(e) = std::fs::remove_file(&output_path) {
                    return Err(format!("Failed to remove existing output file: {}", e).into());
                }
            }
            let mut output = File::create(&output_path)?;
            if platform == &Platform::WindowsX64 {
                PortableExecutable::from(&llrt_binary)?
                    .write_resource(SECTION_NAME, compiled.clone())?
                    .build(&mut output)?;
                output.write_all(MAGIC_NUMBER.as_bytes())?;
            } else if platform == &Platform::LinuxX64 || platform == &Platform::LinuxArm64 {
                Elf::new(&llrt_binary)
                    .append(SECTION_NAME, &compiled, &mut output)?;
                // Linux does not need to write magic number
                // libsui already appends magic number to the output file
                // 1exe6und1e will affect libsui find section
            } else if platform == &Platform::DarwinX64 || platform == &Platform::DarwinArm64 {
                Macho::from(llrt_binary)?
                    .write_section(SECTION_NAME, compiled.clone())?
                    .build_and_sign(&mut output)?;
                output.write_all(MAGIC_NUMBER.as_bytes())?;
            }
        }

        // Print build completion message in green
        println!("\x1b[32mBuild successfully\x1b[0m");
        Ok(())
    }

    // output name for platform
    fn output_path_for_platform(&self, platform: &Platform) -> PathBuf {
        let file_name = match platform {
            Platform::WindowsX64 => format!("{}-{}.exe", self.args.output, platform_to_str(platform)),
            _ => format!("{}-{}", self.args.output, platform_to_str(platform)),
        };

        self.args.directory.join(file_name)
    }
}

fn platform_to_str(platform: &Platform) -> &str {
    match platform {
        Platform::LinuxX64 => "linux-x64",
        Platform::LinuxArm64 => "linux-arm64",
        Platform::DarwinX64 => "darwin-x64",
        Platform::DarwinArm64 => "darwin-arm64",
        Platform::WindowsX64 => "windows-x64",
    }
}

fn platform_to_binary_name(platform: &Platform) -> &str {
    if platform == &Platform::WindowsX64 {
        "llrt.exe"
    } else {
        "llrt"
    }
}

pub fn has_magic_number(platform: &Platform) -> std::io::Result<bool> {
    let path = env::current_exe()?;
    let mut file = File::open(path)?;

    let file_size = file.metadata()?.len() as usize;
    if file_size < 16 {  // ensure enough data to read magic number
        return Ok(false);
    }

    match platform {
        Platform::LinuxX64 | Platform::LinuxArm64 => {
            // for Linux platform, need to detect libsui's magic number
            // libsui appends 16 bytes trailer to the file: magic number(4 bytes) + hash(4 bytes) + size(8 bytes)
            const TRAILER_LEN: i64 = 16;
            file.seek(SeekFrom::End(-TRAILER_LEN))?;
            let mut buf = [0; 4]; // only read magic number part
            file.read_exact(&mut buf)?;
            
            // check magic number (little endian)
            let magic = u32::from_le_bytes(buf);
            Ok(magic == LIBSUI_MAGIC_NUMBER)
        },
        _ => {
            // other platforms use the original string magic number detection method
            let search_area_size = 1024.min(file_size);
            let search_start = file_size.saturating_sub(search_area_size);

            file.seek(SeekFrom::Start(search_start as u64))?;

            let mut buffer = vec![0; search_area_size];
            file.read_exact(&mut buffer)?;

            // search string magic number
            for i in (0..search_area_size - MAGIC_NUMBER.len() + 1).rev() {
                if &buffer[i..i + MAGIC_NUMBER.len()] == MAGIC_NUMBER.as_bytes() {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

pub fn extract_code_binary() -> Option<Vec<u8>> {
    let Some(data) = find_section(SECTION_NAME) else {
        eprintln!("Error finding section");
        return None;
    };

    Some(data.to_vec())
}
