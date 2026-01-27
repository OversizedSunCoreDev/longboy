#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(generic_const_items)]

mod broker;

use anyhow::{Context, Result};
use config::Config;
use longboy::{Server, ServerSession, ThreadRuntime, TokioRuntime};
use longboy_schema::{new_client_to_server_schema, new_server_to_client_schema};
use quinn::{Connection, Endpoint, crypto::rustls::QuicServerConfig};
use rustls::{
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, pem::PemObject},
};
use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;

use crate::broker::{ClientBroker, ServerBroker, SessionBroker};

// This really could be a feature in the toml configuration crate.
#[derive(serde::Deserialize)]
enum LongboyRuntimeType
{
    Tokio,
    Thread,
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
        .add_source(config::Environment::with_prefix("LONGBOY_SERVER"))
        .build()
        .unwrap();

    let config: LongboyServerConfig = settings
        .try_deserialize::<LongboyServerConfig>()
        .expect("Failed to deserialize configuration");

    // Create a cancellation token for the runtime.
    let cancellation_token = CancellationToken::new();
    let _ = run_server_from_config(config, cancellation_token);
}

// Runs the server based on the provided configuration. It uses the tokio runtime by default.
#[tokio::main]
async fn run_server_from_config(config: LongboyServerConfig, cancellation_token: CancellationToken) -> Result<()>
{
    let server_runtime = match config.runtime_type
    {
        LongboyRuntimeType::Tokio =>
        {
            Box::new(TokioRuntime::new(cancellation_token.clone())) as Box<dyn longboy::Runtime>
        }
        LongboyRuntimeType::Thread =>
        {
            Box::new(ThreadRuntime::new(cancellation_token.clone())) as Box<dyn longboy::Runtime>
        }
    };

    let broker = std::sync::Arc::new(SessionBroker::<32>::new());
    let client_broker = ClientBroker::new(broker.clone().into());
    let server_broker = ServerBroker::new(broker.clone().into());

    // Setup the receivers for client-server communication.
    let server_to_client_schema = new_server_to_client_schema();
    let client_to_server_schema = new_client_to_server_schema();

    let server_builder = Server::builder(config.session_capacity, server_runtime)
        .sender::<_, 32, 3>(
            &server_to_client_schema,
            server_broker
        ).unwrap()
        .receiver::<_, 16, 3>(
            &client_to_server_schema,
            client_broker
        ).unwrap();

    // Load TLS certificates. TLS is required.
    let provider = aws_lc_rs::default_provider();
    CryptoProvider::install_default(provider).expect("Failed to install default crypto provider");

    let (cert_chain, key) = {
        let cert_path = std::path::Path::new(&config.public_certificate_path);
        let key_path = std::path::Path::new(&config.private_key_path);
        let key = if key_path.extension().is_some_and(|x| x == "der")
        {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                std::fs::read(key_path).context("failed to read private key file")?,
            ))
        }
        else
        {
            PrivateKeyDer::from_pem_file(key_path).context("failed to read PEM from private key file")?
        };

        let cert_chain = if cert_path.extension().is_some_and(|x| x == "der")
        {
            vec![CertificateDer::from(
                std::fs::read(cert_path).context("failed to read certificate chain file")?,
            )]
        }
        else
        {
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
    let server_endpoint = Endpoint::server(server_config, listen_addr).context("failed to build endpoint")?;

    // create a new logical server instance
    let mut server_instance = server_builder.build();

    // log out some info on server startup
    println!("Longboy server listening on {}", listen_addr);
    // print stats from server endpoint
    println!("Server endpoint stats: {:?}", server_endpoint.stats());


    // start accepting connections
    while let Some(conn) = server_endpoint.accept().await
    {
        // print it
        println!("Incoming connection: {:?}", conn.remote_address());

        // try to accept and handle errors gracefully
        match conn.accept()
        {
            Err(e) =>
            {
                println!("Failed to accept connection: {:?}", e);
                continue;
            }
            Ok(connecting) =>
            {
                match connecting.await
                {
                    Err(e) =>
                    {
                        println!("Failed to establish connection: {:?}", e);
                        continue;
                    }
                    Ok(connection) =>
                    {
                        // handle the connection
                        if let Err(e) = server_handle_connection(connection, &mut server_instance, &broker).await
                        {
                            println!("Error handling connection: {:?}", e);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn server_handle_connection(conn: Connection, server: &mut Server, broker: &std::sync::Arc<SessionBroker<32>>) -> Result<()>
{
    let remote_address = conn.remote_address();
    println!("New connection from {}", remote_address);

    let player_index = broker.next_player_index()?;
    let session_id = broker.allocate_session_id(player_index);
    let cipher_key = 0xdeadbeef;
    let server_session = ServerSession::new(session_id, cipher_key, conn).await?;
    server.register(server_session); // Perhaps this should handle errors in some way? Client ditches mid stream?
    Ok(())
}
