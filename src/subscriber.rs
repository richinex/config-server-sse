use tokio::sync::watch::Receiver;
use crate::config::Config;
use slog::{Logger, info};
use chrono::Local;
use std::sync::Arc;
use serde_json; // For serializing the configuration for logging

pub async fn listen_for_updates(mut receiver: Receiver<(String, Arc<Config>)>, logger: Logger) {
    loop {
        if receiver.changed().await.is_err() {
            info!(logger, "Configuration channel closed"; "time" => Local::now().to_rfc3339());
            break;
        }

        // Correctly destructure the reference to a tuple
        let (env, current_config_ref) = &*receiver.borrow();

        // Serialize and log the configuration along with its environment
        match serde_json::to_string(&current_config_ref.settings) {
            Ok(config_json) => info!(logger, "Configuration updated";
                                     "time" => Local::now().to_rfc3339(),
                                     "environment" => env,
                                     "config" => config_json),
            Err(e) => info!(logger, "Failed to serialize current configuration";
                                     "time" => Local::now().to_rfc3339(),
                                     "environment" => env,
                                     "error" => e.to_string()),
        }
    }
}

