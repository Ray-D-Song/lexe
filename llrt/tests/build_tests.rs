mod build_test {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use pico_args::Arguments;
    use std::str::FromStr;
    use std::ffi::OsString;
    
    // reuse the Platform enum from the build module
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
    fn create_args(args: Vec<String>) -> Arguments {
        Arguments::from_vec(args.into_iter().map(OsString::from).collect())
    }
    
    fn create_temp_file(filename: &str) -> PathBuf {
        let temp_dir = env::temp_dir();
        let file_path = temp_dir.join(filename);
        fs::write(&file_path, "test content").expect("Failed to create temp file");
        file_path
    }

    fn cleanup(paths: &[PathBuf]) {
        for path in paths {
            if path.exists() {
                if path.is_file() {
                    fs::remove_file(path).expect("Failed to remove temp file");
                } else if path.is_dir() {
                    fs::remove_dir_all(path).expect("Failed to remove temp directory");
                }
            }
        }
    }
    
    #[test]
    fn test_missing_required_args() {
        let mut args = create_args(vec![]);
        
        let input = args.opt_value_from_str::<_, PathBuf>("--input")
            .or_else(|_| args.opt_value_from_str::<_, PathBuf>("-i"));
            
        assert!(input.is_ok());
        assert!(input.unwrap().is_none());
    }

    #[test]
    fn test_basic_args() {
        let input_file = create_temp_file("test_input.js");
        
        let mut args = create_args(vec![
            "--input".to_string(),
            input_file.to_string_lossy().to_string(),
            "--directory".to_string(),
            "/tmp/test_dir".to_string(),
            "--platform".to_string(),
            "darwin-arm64".to_string(),
        ]);
        
        let input = args.opt_value_from_str::<_, PathBuf>("--input")
            .or_else(|_| args.opt_value_from_str::<_, PathBuf>("-i"));
        
        assert!(input.is_ok());
        assert_eq!(input.unwrap().unwrap(), input_file);
        
        let dir = args.opt_value_from_str::<_, PathBuf>("--directory")
            .or_else(|_| args.opt_value_from_str::<_, PathBuf>("-d"));
        
        assert!(dir.is_ok());
        assert_eq!(dir.unwrap().unwrap().to_string_lossy(), "/tmp/test_dir");
        
        cleanup(&[input_file]);
    }

    #[test]
    fn test_platform_parsing() {
        let platform_str = "darwin-arm64";
        let platform = Platform::from_str(platform_str);
        assert!(platform.is_ok());
        assert_eq!(platform.unwrap(), Platform::DarwinArm64);
        
        let platforms_str = "darwin-arm64,linux-x64,windows-x64";
        let platforms: Result<Vec<_>, _> = platforms_str.split(',')
            .map(|s| Platform::from_str(s.trim()))
            .collect();
        
        assert!(platforms.is_ok());
        let platforms = platforms.unwrap();
        assert_eq!(platforms.len(), 3);
        assert_eq!(platforms[0], Platform::DarwinArm64);
        assert_eq!(platforms[1], Platform::LinuxX64);
        assert_eq!(platforms[2], Platform::WindowsX64);
        
        let invalid_platform = Platform::from_str("invalid-platform");
        assert!(invalid_platform.is_err());
    }
  }