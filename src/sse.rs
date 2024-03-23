use actix::{Actor, Context, Handler, Message};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use actix_web_lab::sse::{Sse, Event, Data};
use bytestring::ByteString;
use futures::stream::StreamExt;
use std::sync::{Arc, Mutex};
use std::error::Error;
use slog::{info, error, Logger};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use crate::config::Config; // Adjust the path as necessary
use crate::publisher::Publisher; // Adjust the path as necessary
use crate::traits::traits::ConfigUpdateSubscriber; // Adjust the path as necessary


#[derive(Message)]
#[rtype(result = "Option<Arc<Config>>")]
pub struct GetCurrentConfig {
    pub environment: String,
}



impl Handler<GetCurrentConfig> for SseManager {
    type Result = Option<Arc<Config>>;

    fn handle(&mut self, msg: GetCurrentConfig, _: &mut Self::Context) -> Self::Result {
        let publisher_lock = self.publisher.lock().unwrap();
        publisher_lock.get_config_for_env(&msg.environment)
    }
}



// SseManager to manage subscriptions and broadcasting
#[derive(Clone)]
pub struct SseManager {
    publisher: Arc<Mutex<Publisher>>,
}



impl SseManager {
    pub fn new(publisher: Arc<Mutex<Publisher>>) -> Self {
        Self { publisher}
    }

    pub async fn sse_endpoint(
        req: HttpRequest,
        data: web::Data<Arc<Mutex<Publisher>>>,
        logger: web::Data<Logger>,
    ) -> impl Responder {
        let (tx, rx) = mpsc::channel(32);
        let logger = logger.get_ref().clone();

        let env_name = req.match_info().get("environment").unwrap_or("default").to_string();


        {
            let publisher = data.get_ref().lock().unwrap();
            if let Some(current_config) = publisher.get_config_for_env(&env_name) {
                let initial_data = match serde_json::to_string(&*current_config) {
                    Ok(json_string) => Some(Event::Data(Data::new(ByteString::from(json_string)))),
                    Err(e) => {
                        error!(logger, "Failed to serialize initial config data: {}", e);
                        None
                    }
                };

                if let Some(data) = initial_data {
                    if tx.send(data).await.is_err() {
                        error!(logger, "Failed to send initial config data");
                    } else {
                        info!(logger, "Initial config data sent");
                    }
                }
            }
        } // Release the lock early

        let sse_subscriber = Arc::new(SseSubscriber { sender: tx, env: env_name.clone(), logger: Some(logger) });
        data.get_ref().lock().unwrap().subscribe(sse_subscriber);

        let rx_stream = ReceiverStream::new(rx)
            .map(|event| Ok::<_, Box<dyn Error>>(event));

        let sse_response = Sse::from_stream(rx_stream)
            .with_keep_alive(Duration::from_secs(15));

        HttpResponse::Ok()
            .insert_header(("Content-Type", "text/event-stream"))
            .body(sse_response)
    }
}


impl Actor for SseManager {
    type Context = Context<Self>;
}

// SseSubscriber to forward updates to the SSE stream
struct SseSubscriber {
    sender: mpsc::Sender<Event>,
    env: String,
    logger: Option<Logger>,
}

impl SseSubscriber {
    // Optional logging helper functions
    fn log_info(&self, message: &str) {
        if let Some(ref logger) = self.logger {
            info!(logger, "{}", message);
        } else {
            println!("Info: {}", message); // Fallback to println if logger is not available
        }
    }

    fn log_error(&self, message: &str) {
        if let Some(ref logger) = self.logger {
            error!(logger, "{}", message);
        } else {
            eprintln!("Error: {}", message); // Fallback to eprintln if logger is not available
        }
    }
}

impl ConfigUpdateSubscriber for SseSubscriber {
    fn update(&self, env: String, config: Arc<Config>) {
        if env == self.env {
            // Attempt to serialize the config into JSON
            match serde_json::to_string(&*config) {
                Ok(json) => {
                    self.log_info(&format!("Sending update for env: {}", self.env));
                    // Successfully serialized, prepare the data event
                    let data_as_bytestring = ByteString::from(json);
                    let data_event = Data::new(data_as_bytestring);
                    let event = Event::Data(data_event);

                    // Attempt to send the event through the channel
                    if let Err(e) = self.sender.try_send(event) {
                        self.log_error(&format!("Failed to send SSE event: {:?}", e));
                    }
                },
                Err(e) => {
                    // Serialization failed, log the error
                    self.log_error(&format!("Error serializing config: {:?}", e));
                },
            }
        }
    }


    fn env(&self) -> String {
        self.env.clone()
    }
}



#[derive(Message)]
#[rtype(result = "Result<(), String>")]
pub struct UpdateAndBroadcastEnvConfig {
    pub env: String,
    pub config: Config,
}

impl Handler<UpdateAndBroadcastEnvConfig> for SseManager {
    type Result = Result<(), String>;

    fn handle(&mut self, msg: UpdateAndBroadcastEnvConfig, _: &mut Self::Context) -> Self::Result {
        let publisher_lock = self.publisher.lock().unwrap();

        // The call to update_config_for_env not only updates the configuration but also notifies all subscribers
        // about the update, as per the implementation in the Publisher struct.
        let new_config = Arc::new(msg.config);
        publisher_lock.update_config_for_env(msg.env.clone(), new_config);

        Ok(())
    }

}

