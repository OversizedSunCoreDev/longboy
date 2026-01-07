#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(generic_const_items)]
#![feature(map_try_insert)]
#![feature(try_blocks)]

use enum_map::enum_map;
use anyhow::{Context, Result};
use config::Config;
use flume::{Receiver, Sender};
use longboy::{ClientToServerSchema, Factory, Mirroring, Server, ServerSession, ServerToClientSchema, Source, Sink, ThreadRuntime, TokioRuntime};
use quinn::{Connection, Endpoint, crypto::rustls::QuicServerConfig};
use rustls::{
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, pem::PemObject},
};
use std::{net::{SocketAddr, UdpSocket}, sync::Arc};
use tokio_util::sync::CancellationToken;

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

// Server runtime sender and receiver behaviors.
struct ServerToClientSourceFactory
{
    channels: [Receiver<(u32, [u64; 2])>; 32 /* max players per instance */],
}

struct ClientToServerSinkFactory
{
    channel: Sender<(u32, u8, u64)>,
}

struct ClientToServerSink
{
    player_index: u8,
    channel: Sender<(u32, u8, u64)>,
}

struct ServerToClientSource
{
    channel: Receiver<(u32, [u64; 2])>,
}

impl Factory for ServerToClientSourceFactory
{
    type Type = ServerToClientSource;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let player_index = match session_id
        {
            1 => 0,
            2 => 1,
            _ => unreachable!(),
        };

        ServerToClientSource {
            channel: self.channels[player_index].clone(),
        }
    }
}

impl Factory for ClientToServerSinkFactory
{
    type Type = ClientToServerSink;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let player_index = match session_id
        {
            1 => 0,
            2 => 1,
            _ => unreachable!(),
        };

        ClientToServerSink {
            player_index,
            channel: self.channel.clone(),
        }
    }
}

impl Sink<16> for ClientToServerSink
{
    fn handle(&mut self, buffer: &[u8; 16])
    {
        let frame = u32::from_le_bytes(*(<&[u8; 4]>::try_from(&buffer[0..4]).unwrap()));
        let player_input = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[4..12]).unwrap()));
        self.channel.send((frame, self.player_index, player_input)).unwrap();
    }
}

impl Source<32> for ServerToClientSource
{
    fn poll(&mut self, buffer: &mut [u8; 32]) -> bool
    {
        match self.channel.try_recv()
        {
            Ok((frame, player_inputs)) =>
            {
                *(<&mut [u8; 4]>::try_from(&mut buffer[0..4]).unwrap()) = frame.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[4..12]).unwrap()) = player_inputs[0].to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[12..20]).unwrap()) = player_inputs[1].to_le_bytes();
                true
            }
            Err(_) => false,
        }
    }
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

    // Setup the receivers and senders for client-server communication.
    let client_to_server_mapper_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();
    let client_to_server_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();
    let server_to_client_mapper_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();
    let client_to_server_schema = ClientToServerSchema {
        name: "Input",
        mapper_port: client_to_server_mapper_socket.local_addr()?.port(),
        heartbeat_period: 2000,
        port: client_to_server_socket.local_addr()?.port(),
    };
    let server_to_client_schema = ServerToClientSchema {
        name: "State",
        mapper_port: server_to_client_mapper_socket.local_addr()?.port(),
        heartbeat_period: 2000,
    };
    let server_builder = Server::builder(config.session_capacity, server_runtime)
        .sender_with_sockets::<_, 32, 3>(
            &server_to_client_schema,
            server_to_client_mapper_socket,
            enum_map! {
                Mirroring::AudioVideo => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0)))?,
                Mirroring::Background => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0)))?,
                Mirroring::Voice => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0)))?,
            },
            ServerToClientSourceFactory {
                channels: [
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1,
                    flume::unbounded().1
                ],
            },
        )?
        .receiver_with_socket::<_, 16, 3>(
            &client_to_server_schema,
            client_to_server_mapper_socket,
            client_to_server_socket,
            ClientToServerSinkFactory {
                channel: flume::unbounded().0,
            },
        )?;

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
            Err(e) => {
                println!("Failed to accept connection: {:?}", e);
                continue;
            }
            Ok(connecting) => {
                match connecting.await
                {
                    Err(e) => {
                        println!("Failed to establish connection: {:?}", e);
                        continue;
                    }
                    Ok(connection) => {
                        // handle the connection
                        if let Err(e) = server_handle_connection(connection, &mut server_instance).await
                        {
                            println!("Error handling connection: {:?}", e);
                        }
                    }
                }
            },
        }
    }

    Ok(())
}

async fn server_handle_connection(conn: Connection, server: &mut Server) -> Result<()>
{
    let remote_address = conn.remote_address();
    println!("New connection from {}", remote_address);

    let session_id = 0;
    let cipher_key = 0xdeadbeef;
    let server_session = ServerSession::new(session_id, cipher_key, conn).await?;
    server.register(server_session); // Perhaps this should handle errors in some way? Client ditches mid stream?

    loop {
        // Here you would handle incoming requests, manage sessions, etc.
        // For demonstration, we will just break the loop.
        break;
    }

    print!("Server session registered\t\nSession Id: {}\t\nRemote Address: {}\n", session_id, remote_address);
    Ok(())
}
