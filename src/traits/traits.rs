use std::sync::Arc;

use crate::config::Config;

pub trait ConfigUpdateSubscriber {
    fn update(&self, env: String, config: Arc<Config>);
    fn env(&self) -> String; // Add this line
}
