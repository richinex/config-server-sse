use actix::{Actor, Addr};
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use std::{collections:: HashMap, sync::{Arc, Mutex}};
use tokio::sync::watch;

mod config;
mod publisher;
mod subscriber;
mod sse;
mod traits;

use config::{load_configs, Config, ConfigError, ConfigUpdate};
use publisher::Publisher;
use subscriber::listen_for_updates;
use sse::{GetCurrentConfig, SseManager, UpdateAndBroadcastEnvConfig};


use slog::{Drain, Logger, o};
use slog_async::Async;
use slog_json::Json;
use slog_term::{FullFormat, TermDecorator};
use std::path::Path;

fn configure_logging() -> Logger {
    let decorator = TermDecorator::new().build();
    let console_drain = FullFormat::new(decorator).build().fuse();
    let console_drain = Async::new(console_drain).build().fuse();

    let json_drain = Json::new(std::io::stdout())
        .add_default_keys()
        .build().fuse();
    let json_drain = Async::new(json_drain).build().fuse();

    Logger::root(slog::Duplicate::new(console_drain, json_drain).fuse(), o!())
}


async fn update_config(
    sse_manager: web::Data<Addr<SseManager>>,
    logger: web::Data<Logger>,
    payload: web::Json<ConfigUpdate>,
) -> impl Responder {
    let update_message = payload.into_inner();

    let new_config = Config {
        settings: update_message.settings,
    };

    // Retrieve the current configuration for comparison by sending the GetCurrentConfig message
    // with the environment specified in the update_message
    let current_config_future = sse_manager.send(GetCurrentConfig {
        environment: update_message.env.clone(), // Clone the environment string for the message
    });

    // Since .send() is asynchronous, wait for the future to resolve
    let current_config_result = current_config_future.await;

    match current_config_result {
        Ok(Some(current_config)) => {
            // Successfully retrieved the current configuration, proceed to compare
            if Arc::try_unwrap(current_config).unwrap_or_else(|arc| (*arc).clone()) == new_config {
                // Configurations are the same, no update needed
                return HttpResponse::Ok().body("No changes in configuration.");
            }
        },
        Ok(None) => {
            // Current config could not be found for the given environment
            // Logging this scenario for now
            slog::warn!(logger.get_ref(), "No current configuration found for the specified environment"; "environment" => %update_message.env);

        },
        Err(e) => {
            // Error occurred while trying to retrieve the current config
            slog::warn!(logger.get_ref(), "Failed to retrieve current configuration"; "error" => %e);
        },
    }

    // If the configurations are different or current config could not be retrieved, proceed with the update
    match sse_manager.send(UpdateAndBroadcastEnvConfig {
        env: update_message.env,
        config: new_config,
    }).await {
        Ok(result) => match result {
            Ok(_) => HttpResponse::Ok().body("Configuration updated and broadcasted successfully."),
            Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
        },
        Err(_) => HttpResponse::InternalServerError().body("Actor message sending failed."),
    }
}


/// Fetches and returns the configuration for the specified environment.
/// If the environment configuration is not found, it returns a 404 response.
async fn get_config_endpoint(
    publisher: web::Data<Arc<Mutex<Publisher>>>,
    logger: web::Data<Logger>,
    path: web::Path<String>, // Change here, use the Path directly
) -> impl actix_web::Responder {
    let env = path.into_inner(); // Extract the string from Path<String>

    let publisher_lock = publisher.lock().unwrap();

    // Attempt to fetch the configuration for the specified environment.
    match publisher_lock.get_config_for_env(&env) {
        Some(config) => {
            // Found the configuration for the environment, log and return it.
            slog::info!(logger.get_ref(), "Fetching configuration"; "env" => &env);
            HttpResponse::Ok().json(&config.settings)

        },
        None => {
            // Configuration for the specified environment not found, log and respond with 404.
            slog::info!(logger.get_ref(), "Configuration not found"; "env" => &env);
            HttpResponse::NotFound().body(format!("Configuration not found for environment: {}", env))
        }
    }
}


async fn get_config_history(
    publisher: web::Data<Arc<Mutex<Publisher>>>,
    path: web::Path<String>,
) -> impl actix_web::Responder {
    let env = path.into_inner();
    let publisher_lock = publisher.lock().expect("Failed to lock publisher mutex");

    match publisher_lock.get_config_history_for_env(&env) {
        Ok(history) => HttpResponse::Ok().json(history), // Direct serialization of the history
        Err(ConfigError::NotFound(_)) => HttpResponse::NotFound().body(format!("Configuration history not found for environment: {}", env)),
        // Explicitly handle other errors if necessary
        Err(e) => {
            eprintln!("Error retrieving configuration history: {:?}", e); // Log or handle other errors as needed
            HttpResponse::InternalServerError().body("Unexpected error occurred")
        },
    }
}


async fn get_specific_config_version(
    publisher: web::Data<Arc<Mutex<Publisher>>>,
    info: web::Path<(String, usize)>, // Tuple path for environment and version
) -> impl actix_web::Responder {
    let (env, version) = info.into_inner();
    let publisher_lock = publisher.lock().unwrap(); // Consider handling this potential panic

    match publisher_lock.get_specific_config_version_for_env(&env, version) {
        Ok(config) => HttpResponse::Ok().json(&config.settings), // Assuming Config is serializable
        Err(ConfigError::NotFound(_)) => HttpResponse::NotFound().body(format!("Configuration not found for environment: {}", env)),
        Err(ConfigError::OutOfBounds(_)) => HttpResponse::BadRequest().body(format!("Invalid version number: {} for environment: {}", version, env)),
        // Handle other specific errors, if any
        Err(_) => HttpResponse::InternalServerError().body("Unexpected error occurred"),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let root_logger = configure_logging();
    slog::info!(root_logger, "Starting HTTP server for configuration management");

    // Load configurations for all environments
    let configs = load_configs(Path::new("./configs")).await
        .expect("Failed to load configurations")
        .into_iter()
        // Directly use the Arc<Config> without additional wrapping
        .collect::<HashMap<String, Arc<Config>>>();

    // Assuming `configs` is correctly a HashMap<String, Arc<Config>>
    let initial_environment = "dev".to_string();
    let initial_config = configs.get(&initial_environment)
        .map(|config| Arc::clone(config)) // Clone the Arc if present
        .unwrap_or_else(|| {
            // Log that we are falling back to an empty default configuration
            slog::warn!(root_logger, "Default configuration 'dev.yml' is missing. Falling back to an empty configuration.");
            Arc::new(Config { settings: Default::default() }) // Create a new Arc for the default config
        });

    // Now, initial_config is an Arc<Config> as expected
    let (tx, rx) = watch::channel::<(String, Arc<Config>)>((initial_environment.clone(), initial_config.clone())); // Clone here to keep ownership as well
        // Preparing a Publisher with the loaded configurations
    let publisher = Arc::new(Mutex::new(Publisher::new(
        tx, // Correctly pass the sender here
        configs,
        initial_environment,
        Some(root_logger.clone())
    )));

     // Instantiating the SSE manager with the publisher
    let sse_manager = SseManager::new(publisher.clone()).start();

    // Clone the logger before passing to tokio::spawn to retain ownership for later use
    let subscriber_logger = root_logger.clone();
    tokio::spawn(async move {
        listen_for_updates(rx, subscriber_logger).await; // Use cloned logger
    });


    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(root_logger.clone()))
            .app_data(web::Data::new(publisher.clone()))
            .app_data(web::Data::new(sse_manager.clone()))
            .route("/sse/{environment}", web::get().to(SseManager::sse_endpoint))
            .route("/config/{env}", web::get().to(get_config_endpoint)) // Fetch configuration by environment endpoint
            .route("/config", web::post().to(update_config)) // Update configuration endpoint
            .route("/config/history/{env}", web::get().to(get_config_history))
            .route("/config/version/{env}/{version}", web::get().to(get_specific_config_version))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

