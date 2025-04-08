use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

/**
 * Build::validate_args(args)
 *     .run_build()
 */

#[derive(Debug, Clone, ValueEnum)]
enum Platform {
    #[value(name = "linux-x64")]
    LinuxX64,
    #[value(name = "linux-arm64")]
    LinuxArm64,
    #[value(name = "darwin-x64")]
    MacosX64,
    #[value(name = "darwin-arm64")]
    MacosArm64,
    #[value(name = "windows-x64")]
    WindowsX64,
    #[value(name = "windows-arm64")]
    WindowsArm64,
}

#[derive(Debug, Parser)]
struct BuildArgs {
    /// Input file (required)
    #[arg(short, long, required = true)]
    input: PathBuf,

    /// Output file name (optional, default: input-<platform>)
    #[arg(short, long)]
    output: Option<String>,

    /// Output directory (optional, default: ./dist)
    #[arg(short, long, default_value = "dist")]
    directory: PathBuf,

    /// Target platform(s), comma-separated
    #[arg(short, long, value_delimiter = ',', default_value_t = Platform::current())]
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
        } else if cfg!(target_os = "darwin") {
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

impl LexeBuild {
    /// Validate build arguments and return a Build instance
    pub fn validate_args(args: BuildArgs) -> Result<Self, String> {
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
    pub fn run_build(self) -> Result<(), String> {
        println!("Starting build process...");

        for platform in &self.args.platform {
            let output_name = self.output_name_for_platform(platform);
            let output_path = self.args.directory.join(output_name);

            println!("Building for {:?} -> {}", platform, output_path.display());

            // Here you would actually implement:
            // 1. Compilation for the platform
            // 2. Output the executable to the specified path
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
