# AyaDance Server

AyaDance Server is a backend server for VRChat world [Aya Dance](https://vrchat.com/home/world/wrld_9ad22e66-8f3a-443e-81f9-87c350ed5113).


## Getting Started

### Prerequisites

- Rust (latest nightly version)
- Docker (optional, for containerized deployment)

### Installation

1. Clone the repository:
    ```bash
    git clone https://github.com/ClownpieceStripedAbyss/aya-dance-server.git
    cd aya-dance-server
    ```

2. Build the project:
    ```bash
    cargo build --release
    ```

3. Run the server:
    ```bash
    cargo run --release
    ```

### Docker Deployment

1. Build the Docker image:
    ```bash
    docker build . -f docker/aya-dance/Dockerfile
    ```

2. Run with docker compose, see `docker/aya-dance/compose.yml` for example.

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

## License

This project is licensed under the MIT License.
