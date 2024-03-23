
# Configuration Management Server

This server application provides a solution for managing and broadcasting configuration updates in real-time. It uses Server-Sent Events (SSE) to enable clients to listen for updates to configurations across different environments.

## Features

- Real-time configuration updates using SSE.
- Endpoints for fetching current configurations, updating configurations, and accessing configuration history.
- Logging with both console and JSON outputs for effective monitoring.

## Technologies

- Rust programming language.
- Actix and Actix-Web for the web server and actor model concurrency.
- Tokio for asynchronous runtime.
- Slog for flexible logging.

## Getting Started

### Prerequisites

- Rust and Cargo installed on your system.
- Basic knowledge of Rust and asynchronous programming.

### Running the Server

1. Clone the repository to your local machine.
2. Navigate to the project directory.
3. Run `cargo run` to start the server.
4. The server will start listening on `127.0.0.1:8080`.

## Endpoints

- `GET /sse/{environment}`: Subscribe to real-time updates for a specific environment.
- `GET /config/{env}`: Fetch the current configuration for a specific environment.
- `POST /config`: Update the configuration for a specific environment.
- `GET /config/history/{env}`: Get the configuration update history for a specific environment.
- `GET /config/version/{env}/{version}`: Fetch a specific version of the configuration for an environment.

## Configuration

Configure logging and other settings by editing the `config.rs` and `logger.rs` modules as needed.

## License

This project is licensed under the MIT License - see the LICENSE.md file for details.
