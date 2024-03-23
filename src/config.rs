use serde::{Deserialize, Serialize};
use serde_yaml::{Value, Error as YamlError};
use std::collections::{HashMap, BTreeMap};
use tokio::fs;
use tokio::io::AsyncReadExt;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub enum ConfigError {
    IoError(std::io::Error),
    YamlError(serde_yaml::Error),
    DirectoryReadError(std::io::Error),
    NotFound(String), // Indicates a configuration or version was not found
    OutOfBounds(String), // Indicates the requested version number is out of bounds
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> ConfigError {
        match err.kind() {
            std::io::ErrorKind::NotFound => ConfigError::NotFound(err.to_string()),
            _ => ConfigError::IoError(err),
        }
    }
}

impl From<YamlError> for ConfigError {
    fn from(err: YamlError) -> ConfigError {
        ConfigError::YamlError(err)
    }
}



#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub settings: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigUpdate {
    pub env: String,
    // Directly use settings here
    pub settings: BTreeMap<String, Value>,
}

pub async fn load_configs(config_dir: &Path) -> Result<HashMap<String, Arc<Config>>, ConfigError> {
    let mut configs = HashMap::new();
    let read_dir_result = fs::read_dir(config_dir).await;

    // Check if the error is due to the directory not being found
    let mut paths = match read_dir_result {
        Ok(paths) => paths,
        Err(err) => return match err.kind() {
            std::io::ErrorKind::NotFound => Err(ConfigError::NotFound(err.to_string())),
            _ => Err(ConfigError::DirectoryReadError(err)),
        },
    };

    while let Some(path_entry) = paths.next_entry().await.map_err(ConfigError::IoError)? {
        let path = path_entry.path();
        if let Some(extension) = path.extension().and_then(|s| s.to_str()) {
            if extension == "yml" || extension == "yaml" {
                if let Some(filename) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Some((_, env)) = filename.rsplit_once('-') {
                        let mut file = fs::File::open(&path).await.map_err(ConfigError::IoError)?;
                        let mut contents = String::new();
                        file.read_to_string(&mut contents).await.map_err(ConfigError::IoError)?;
                        let settings: BTreeMap<String, Value> = serde_yaml::from_str(&contents).map_err(ConfigError::YamlError)?;
                        configs.insert(env.to_string(), Arc::new(Config { settings }));
                    } else {
                        eprintln!("Ignoring file `{}` as it does not follow the `-<env>` naming convention.", filename);
                    }
                }
            }
        }
    }

    Ok(configs)
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs::File;
    use std::io::Write;

    async fn write_temp_config_file(dir: &Path, file_name: &str, content: &str) {
        let file_path = dir.join(file_name);
        let mut file = File::create(file_path).expect("Failed to create temp config file");
        writeln!(file, "{}", content).expect("Failed to write to temp config file");
    }

    #[tokio::test]
    async fn test_load_configs_success() {
        let temp_dir = tempdir().expect("Failed to create a temporary directory");
        let config_dir = temp_dir.path();

        // Mock configuration content
        let config_content = "setting: 'value'";
        // Write temporary config files
        write_temp_config_file(config_dir, "application-dev.yml", config_content).await;
        write_temp_config_file(config_dir, "application-prod.yml", "setting: 'value2'").await;

        // Load configurations
        let configs_result = load_configs(config_dir).await;
        // Verify that loading configurations succeeds
        assert!(configs_result.is_ok(), "Expected successful loading of configurations");


        let configs = configs_result.unwrap();
        assert!(configs.contains_key("dev"), "Should contain config for 'dev' environment");
        assert!(configs.contains_key("prod"), "Should contain config for 'prod' environment");

        // Verify specific settings
        assert_eq!(configs["dev"].settings.get("setting").unwrap(), &Value::from("value"));
        assert_eq!(configs["prod"].settings.get("setting").unwrap(), &Value::from("value2"));
    }

    #[tokio::test]
    async fn test_load_configs_ignore_invalid_files() {
        let temp_dir = tempdir().expect("Failed to create a temporary directory");
        let config_dir = temp_dir.path();

        // Write a file that does not follow the naming convention
        write_temp_config_file(config_dir, "invalidfile.txt", "ignored: 'true'").await;

        // Load configurations
        let configs_result = load_configs(config_dir).await;
        assert!(configs_result.is_ok(), "Expected successful loading of configurations");

        let configs = configs_result.unwrap();
        assert!(configs.is_empty(), "Should ignore files that do not follow the naming convention");
    }

    #[tokio::test]
    async fn test_load_configs_invalid_yaml() {
        let temp_dir = tempdir().expect("Failed to create a temporary directory");
        let config_dir = temp_dir.path();

        // Write a temporary config file with invalid YAML content
        write_temp_config_file(config_dir, "application-broken.yml", "setting: : 'invalid").await;

        // Attempt to load configurations
        let configs_result = load_configs(config_dir).await;
        // Verify that loading configurations results in a YAML error
        assert!(configs_result.is_err(), "Expected a YAML parsing error");

        // Here, you could further assert the error type if you want to be more specific
        if let Err(ConfigError::YamlError(_)) = configs_result {
        } else {
            panic!("Expected a YamlError");
        }
    }

    #[tokio::test]
    async fn test_load_configs_not_found() {
        // Create a temporary directory and then remove it to simulate a non-existent directory scenario
        let temp_dir = tempdir().expect("Failed to create a temporary directory");
        let non_existent_dir = temp_dir.path().join("nonexistent");
        std::fs::create_dir_all(&non_existent_dir).expect("Failed to create a subdirectory");
        std::fs::remove_dir_all(&non_existent_dir).expect("Failed to remove the subdirectory");

        // Attempt to load configurations from the non-existent directory
        let configs_result = load_configs(&non_existent_dir).await;

        // Verify that loading configurations results in a "not found" or similar error
        assert!(configs_result.is_err(), "Expected a 'not found' error when loading from a non-existent directory");

        // Optionally, assert on the specific error type if your error handling distinguishes "not found" errors
        if let Err(ConfigError::NotFound(_)) = configs_result {
            // Test passes for a NotFound error
        } else if let Err(ConfigError::DirectoryReadError(io_err)) = configs_result {
            assert_eq!(io_err.kind(), std::io::ErrorKind::NotFound, "Expected a NotFound IO error");
        } else {
            panic!("Expected a NotFound or DirectoryReadError due to non-existent directory");
        }
    }


}
