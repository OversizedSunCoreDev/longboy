use longboy::{Server, ServerSession, ThreadRuntime, TokioRuntime};
use config::Config;
use quinn::{Endpoint, crypto::rustls::QuicServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, pem::PemObject};
use std::{net::SocketAddr, sync::Arc};
use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

// This really could be a feature in the toml configuration crate.
#[derive(serde::Deserialize)]
enum LongboyRuntimeType {
    Tokio,
    Thread
}

#[derive(serde::Deserialize)]
struct LongboyServerConfig
{
    // Define configuration fields here
    // e.g., session_capacity: usize,
    //       runtime: enum of tokio or async-std,
    session_capacity: usize,
    runtime_type: LongboyRuntimeType,
    public_certificate_path: String,
    private_key_path: String,
    listen_address: Option<SocketAddr>,
}

/// Entry point of the application.
///
/// This function collects configuration settings and starts the server.
fn main()
{
    let base_config_dir = std::env::var("LONGBOY_CONFIG_DIR").unwrap_or_else(|_| ".".to_string());
    // Setup the configuration builder for the server. Let environment variables take the highest precedence.
    let settings = Config::builder()
        .add_source(config::File::with_name(&format!("{}/longboy", base_config_dir)).required(false))
        .add_source(config::Environment::with_prefix("LONGBOY"))
        .build()
        .unwrap();

    let config: LongboyServerConfig = settings.try_deserialize::<LongboyServerConfig>().expect("Failed to deserialize configuration");

    // Create a cancellation token for the runtime.
    let cancellation_token = CancellationToken::new();
    let _ = run_server_from_config(config, cancellation_token);
}

// Runs the server based on the provided configuration. It uses the tokio runtime by default.
#[tokio::main]
async fn run_server_from_config(config: LongboyServerConfig, cancellation_token: CancellationToken) -> Result<()>
{
    let server_runtime = match config.runtime_type {
        LongboyRuntimeType::Tokio => Box::new(TokioRuntime::new(cancellation_token.clone())) as Box<dyn longboy::Runtime>,
        LongboyRuntimeType::Thread => Box::new(ThreadRuntime::new(cancellation_token.clone())) as Box<dyn longboy::Runtime>,
    };
    let server_builder = Server::builder(config.session_capacity, server_runtime);

    // Load TLS certificates. TLS is required.
    let (cert_chain, key) = {
        let cert_path = std::path::Path::new(&config.public_certificate_path);
        let key_path = std::path::Path::new(&config.private_key_path);
        let key = if key_path.extension().is_some_and(|x| x == "der") {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                std::fs::read(key_path).context("failed to read private key file")?,
            ))
        } else {
            PrivateKeyDer::from_pem_file(key_path)
                .context("failed to read PEM from private key file")?
        };

        let cert_chain = if cert_path.extension().is_some_and(|x| x == "der") {
            vec![CertificateDer::from(
                std::fs::read(cert_path).context("failed to read certificate chain file")?,
            )]
        } else {
            CertificateDer::pem_file_iter(cert_path)
                .context("failed to read PEM from certificate chain file")?
                .collect::<Result<_, _>>()
                .context("invalid PEM-encoded certificate")?
        };

        (cert_chain, key)
    };

    // setup listen address
    let listen_addr = config.listen_address.unwrap_or_else(|| "[::1]:4433".parse().unwrap());

    // Now use Quinn (HTTP/3 (QUIP)) to start a listening endpoint.
    let server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));
    let server_endpoint = Endpoint::server(
        server_config,
        listen_addr,
    ).context("failed to build endpoint")?;

    // create a new logical server instance
    let mut server_instance = server_builder.build();

    // start accepting connections
    while let Some(conn) = server_endpoint.accept().await {
        handle_connection(conn, &mut server_instance).await?;
    };

    Ok(())
}

async fn handle_connection(conn: quinn::Incoming, server: &mut Server) -> Result<()> {
    let connection = conn.await?;
    println!("New connection from {}", connection.remote_address());
    
    let session_id = 0;
    let cipher_key = 0xdeadbeef;
    let server_session = ServerSession::new(session_id, cipher_key, connection).await?;
    server.register(server_session);
    Ok(())
}
