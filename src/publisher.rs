use actix::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::watch::Sender;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use slog::{Logger, info, error};
use chrono::{DateTime, Utc};
use serde_json; // For serialization
use crate::config::{Config, ConfigError};
use crate::traits::traits::ConfigUpdateSubscriber;
use serde::ser::SerializeStruct;
// Assuming ConfigVersion and ConfigError are defined as previously described

// Temporary struct that mirrors ConfigVersion but without Arc
#[derive(Deserialize)]
struct ConfigVersionHelper {
    config: Config,
    version: usize,
    timestamp: DateTime<Utc>,
}

#[derive(Message, Serialize, Deserialize, Clone)]
#[rtype(result = "()")]
pub struct ConfigUpdateMessage {
    pub config: Config,
}

// Implement Deserialize for ConfigVersion manually
#[derive(Debug, Clone)]
pub struct ConfigVersion {
    pub config: Arc<Config>,
    pub version: usize,
    pub timestamp: DateTime<Utc>,
}

impl<'de> Deserialize<'de> for ConfigVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize into the helper struct
        let helper = ConfigVersionHelper::deserialize(deserializer)?;
        // Convert to ConfigVersion, wrapping config in an Arc
        Ok(ConfigVersion {
            config: Arc::new(helper.config),
            version: helper.version,
            timestamp: helper.timestamp,
        })
    }
}

impl Serialize for ConfigVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ConfigVersion", 3)?;
        // Assuming Config implements Serialize; serialize the dereferenced Config
        state.serialize_field("config", &*self.config)?;
        state.serialize_field("version", &self.version)?;
        state.serialize_field("timestamp", &self.timestamp)?;
        state.end()
    }
}
pub struct Publisher {
    sender: Sender<(String, Arc<Config>)>,
    configs: Mutex<HashMap<String, Vec<ConfigVersion>>>,
    current_environment: Mutex<String>,
    pub logger: Option<Logger>,
    subscribers: Mutex<Vec<Arc<dyn ConfigUpdateSubscriber + Send + Sync>>>,
}

impl Publisher {
    pub fn new(sender: Sender<(String, Arc<Config>)>, initial_configs: HashMap<String, Arc<Config>>, initial_environment: String, logger: Option<Logger>) -> Self {
        // Explicitly specify the type of configs_history as HashMap<String, Vec<ConfigVersion>>
        let configs_history: HashMap<String, Vec<ConfigVersion>> = initial_configs.into_iter().map(|(env, config)| {
            let history = vec![ConfigVersion {
                config,
                version: 1,
                timestamp: Utc::now(),
            }];
            (env, history)
        }).collect();

        // Immediately broadcast the initial environment's configuration, if available
        let initial_config = configs_history.get(&initial_environment)
            .and_then(|history| history.first())
            .map(|cv| cv.config.clone())
            .unwrap_or_else(|| Arc::new(Config { settings: Default::default() }));

        let _ = sender.send((initial_environment.clone(), initial_config));

        Self {
            sender,
            configs: Mutex::new(configs_history),
            current_environment: Mutex::new(initial_environment),
            logger,
            subscribers: Mutex::new(Vec::new()),
        }
    }

        pub fn subscribe(&self, subscriber: Arc<dyn ConfigUpdateSubscriber + Send + Sync>) {
            let mut subscribers = self.subscribers.lock().unwrap();
            let env = subscriber.env(); // Call the new `env` method
            println!("Subscribed new SSESubscriber for env: {}", env);
            subscribers.push(subscriber);
        }



    pub fn update_config_for_env(&self, env: String, new_config: Arc<Config>) {
        let mut configs_lock = self.configs.lock().unwrap();
        let history = configs_lock.entry(env.clone()).or_insert_with(Vec::new);
        let new_version = ConfigVersion {
            config: new_config.clone(),
            version: history.len() + 1,
            timestamp: Utc::now(),
        };
        history.push(new_version);

        // Logging the update
        if let Some(ref logger) = self.logger {
            info!(logger, "Configuration updated for environment"; "env" => &env, "version" => history.len());
            match serde_json::to_string(&*new_config) {
                Ok(config_str) => info!(logger, "Adding new configuration version"; "config" => config_str),
                Err(e) => error!(logger, "Failed to serialize config for logging"; "error" => e.to_string()),
            }
        }

        // Notify subscribers
        let subscribers = self.subscribers.lock().unwrap();
        for subscriber in subscribers.iter() {
            subscriber.update(env.clone(), new_config.clone());
        }

        let current_env = self.current_environment.lock().unwrap();
        if *current_env == env {
            let _ = self.sender.send((env, new_config));
        }
    }

    // Retrieve the latest configuration for a given environment
    pub fn get_config_for_env(&self, env: &str) -> Option<Arc<Config>> {
        self.configs.lock().unwrap().get(env).and_then(|history| history.last()).map(|cv| cv.config.clone())
    }

    // Retrieve the entire configuration history for a given environment
    pub fn get_config_history_for_env(&self, env: &str) -> Result<Vec<ConfigVersion>, ConfigError> {
        let configs_lock = self.configs.lock().unwrap(); // Use unwrap() for simplicity; consider proper error handling for production code
        configs_lock.get(env)
            .cloned()
            .ok_or_else(|| ConfigError::NotFound(format!("No configuration history found for environment: {}", env)))
    }

    // Retrieve a specific configuration version for a given environment
    pub fn get_specific_config_version_for_env(&self, env: &str, version: usize) -> Result<Arc<Config>, ConfigError> {
        let configs_lock = self.configs.lock().unwrap(); // Again, unwrap() used for simplicity
        let history = configs_lock.get(env)
            .ok_or_else(|| ConfigError::NotFound(format!("Environment not found: {}", env)))?;

        history.get(version - 1)
            .map(|cv| cv.config.clone())
            .ok_or_else(|| ConfigError::OutOfBounds(format!("Version {} not found for environment: {}", version, env)))
    }

}


#[cfg(test)]
mod config_loading_tests {
    use crate::config::load_configs;
    use tempfile::tempdir;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use serde_yaml::Value;

    // Helper to write a mock configuration file
    fn write_mock_config_file(dir: &Path, file_name: &str, content: &str) {
        let file_path = dir.join(file_name);
        let mut file = File::create(file_path).expect("Failed to create mock config file");
        writeln!(file, "{}", content).expect("Failed to write to mock config file");
    }

    #[tokio::test]
    async fn test_loading_configs_from_files() {
        let temp_dir = tempdir().expect("Failed to create a temporary directory");

        // Write mock configuration files
        write_mock_config_file(temp_dir.path(), "application-dev.yml", "setting: value");
        write_mock_config_file(temp_dir.path(), "application-prod.yml", "setting: value2");

        // Assuming `load_configs` is an async function that loads configs from a given directory
        let configs = load_configs(temp_dir.path()).await.expect("Failed to load configurations");

        // Validate loaded configurations
        assert!(configs.contains_key("dev"));
        assert!(configs.contains_key("prod"));
        assert_eq!(configs["dev"].settings.get("setting"), Some(&Value::from("value")));
        assert_eq!(configs["prod"].settings.get("setting"), Some(&Value::from("value2")));

        // Cleanup is handled automatically when `temp_dir` goes out of scope
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;
    use slog::{o, Drain, Logger};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, Mutex};
    use tokio::sync::watch::{self, Receiver};

    struct MockSubscriber {
        env: String,
        received_updates: Mutex<Vec<Arc<Config>>>, // Track received updates
    }

    impl MockSubscriber {
        fn new(env: &str) -> Self {
            MockSubscriber {
                env: env.to_string(),
                received_updates: Mutex::new(vec![]),
            }
        }

         // Allows checking if any updates were received
        fn has_received_updates(&self) -> bool {
            !self.received_updates.lock().unwrap().is_empty()
        }
    }

    impl ConfigUpdateSubscriber for MockSubscriber {
        fn update(&self, env: String, config: Arc<Config>) {
            if env == self.env {
                self.received_updates.lock().unwrap().push(config);
            }
        }

        fn env(&self) -> String {
            self.env.clone()
        }
    }


    fn mock_config(setting_value: &str) -> Arc<Config> {
        let mut settings = BTreeMap::new();
        settings.insert("setting".to_string(), Value::from(setting_value));
        Arc::new(Config { settings })
    }

    fn setup_publisher() -> (Publisher, Receiver<(String, Arc<Config>)>, Logger) {
        let (tx, rx) = watch::channel((String::new(), Arc::new(Config { settings: BTreeMap::new() })));
        let logger = Logger::root(slog::Discard.fuse(), o!());

        // Explicitly create an initial configuration with the expected settings.
        let mut initial_config_settings = BTreeMap::new();
        initial_config_settings.insert("setting".to_string(), Value::from("initial value"));
        let initial_config = Arc::new(Config { settings: initial_config_settings });

        // Use this initial configuration for the "default" environment.
        let mut initial_configs = HashMap::new();
        initial_configs.insert("default".to_string(), initial_config);

        // Initialize the Publisher with the initial configurations.
        let publisher = Publisher::new(tx, initial_configs, "default".to_string(), Some(logger.clone()));

        (publisher, rx, logger)
    }

    #[tokio::test]
    async fn publisher_initializes_correctly() {
        let (publisher, mut rx, _logger) = setup_publisher();

        // Define the expected initial environment and the expected setting value.
        let expected_initial_env = "default".to_string();
        let expected_initial_setting_value = "initial value";

        // Ensure the initial configuration is broadcast correctly.
        // Wait for the receiver to catch up with the broadcast update.
        assert!(rx.changed().await.is_ok(), "Expected to receive an initial configuration broadcast.");

        let (initial_env_broadcast, initial_config_broadcast) = rx.borrow().clone();

        // Check if the initial environment matches the expected value.
        assert_eq!(initial_env_broadcast, expected_initial_env, "The initial environment should match.");

        // Check if the initial configuration's setting matches the expected value.
        // Here, we directly check the value within the configuration to ensure correctness.
        let setting_value = initial_config_broadcast.settings.get("setting")
                            .and_then(|val| val.as_str()); // Assuming settings are stored as `serde_yaml::Value`.
        assert_eq!(setting_value, Some(expected_initial_setting_value), "The initial config's setting should match the expected value.");

        // Test that a new subscriber does not receive the initial configuration upon subscription.
        // This assumes the `MockSubscriber` tracks received updates in a way that can be checked.
        let test_subscriber = Arc::new(MockSubscriber::new("test_env"));
        publisher.subscribe(test_subscriber.clone());

        // Check if the test subscriber received any updates. It should not receive the initial configuration upon subscription.
        // This assertion depends on the implementation of `MockSubscriber` to track and expose received updates for verification.
        assert!(!test_subscriber.has_received_updates(), "Subscriber should not receive updates upon initialization.");
    }


    #[tokio::test]
    async fn subscriber_receives_updates() {
        let (publisher, _rx, _logger) = setup_publisher();
        let subscriber = Arc::new(MockSubscriber::new("dev"));

        publisher.subscribe(subscriber.clone());

        let new_config = mock_config("updated value");
        publisher.update_config_for_env("dev".to_string(), new_config.clone());

        let received_updates = subscriber.received_updates.lock().unwrap();
        assert!(!received_updates.is_empty(), "Subscriber should receive update");
        assert_eq!(received_updates[0].settings.get("setting"), Some(&Value::from("updated value")));
    }

    #[tokio::test]
    async fn update_config_updates_internal_state() {
        let (publisher, _rx, _logger) = setup_publisher();

        let new_config = mock_config("new value");
        publisher.update_config_for_env("test_env".to_string(), new_config.clone());

        let config = publisher.get_config_for_env("test_env").unwrap();
        assert_eq!(config.settings.get("setting"), Some(&Value::from("new value")));
    }

    #[tokio::test]
    async fn get_config_history_returns_all_versions() {
        let (publisher, _rx, _logger) = setup_publisher();

        publisher.update_config_for_env("test_env".to_string(), mock_config("value1"));
        publisher.update_config_for_env("test_env".to_string(), mock_config("value2"));

        let history = publisher.get_config_history_for_env("test_env").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].config.settings.get("setting"), Some(&Value::from("value1")));
        assert_eq!(history[1].config.settings.get("setting"), Some(&Value::from("value2")));
    }
}
