#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(generic_const_items)]
#![feature(unboxed_closures)]

use std::{
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use config::Config;
use longboy::{Client, ClientSession, Sink, Source};
use longboy_schema::{new_client_to_server_schema, new_server_to_client_schema};
use quinn::{ClientConfig, Endpoint};
use rustls::{
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, pem::PemObject},
};
use rustls_native_certs::load_native_certs;
use tokio::{time::sleep};
use tokio_util::sync::CancellationToken;

#[derive(serde::Deserialize)]
struct LongboyClientConfig
{
    certificate_trust_store: Option<String>,
    server_address: SocketAddr,
    server_name: String,
}

struct ServerToClientSink
{
    channel: flume::Sender<(u32, u8, u64)>,
}

struct ClientToServerSource
{
    channel: flume::Receiver<(u32, u64)>,
}

impl Source<16> for ClientToServerSource
{
    fn poll(&mut self, buffer: &mut [u8; 16]) -> bool
    {
        let msg = self.channel.recv();
        match msg
        {
            Ok((frame, val)) =>
            {
                print!("Sending {}", val);
                *(<&mut [u8; 4]>::try_from(&mut buffer[0..4]).unwrap()) = frame.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[4..12]).unwrap()) = u64::from(val).to_le_bytes();
                true
            }
            _ => false,
        }
    }
}

impl Sink<32> for ServerToClientSink
{
    fn handle(&mut self, buffer: &[u8; 32])
    {
        let frame = u32::from_le_bytes(*(<&[u8; 4]>::try_from(&buffer[0..4]).unwrap()));
        let player_id = u8::from_le_bytes(*(<&[u8; 1]>::try_from(&buffer[4..5]).unwrap()));
        let player_input = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[5..13]).unwrap()));
        self.channel.send((frame, player_id, player_input)).unwrap();
    }
}

fn main()
{
    let base_config_dir = std::env::var("LONGBOY_CONFIG_DIR").unwrap_or_else(|_| ".".to_string());
    // Setup the configuration builder for the server. Let environment variables take the highest precedence.
    let settings = Config::builder()
        .add_source(config::File::with_name(&format!("{}/longboy", base_config_dir)).required(false))
        .add_source(config::Environment::with_prefix("LONGBOY_CLIENT"))
        .build()
        .unwrap();
    let config: LongboyClientConfig = settings
        .try_deserialize::<LongboyClientConfig>()
        .expect("Failed to deserialize configuration");
    if let Err(err) = run_client_from_config(config)
    {
        eprintln!("Error running longboy client: {err:#}");
    }
}

#[tokio::main]
async fn run_client_from_config(config: LongboyClientConfig) -> anyhow::Result<()>
{
    // load rustls default crypto provider with aws-lc-rs backend
    let provider = aws_lc_rs::default_provider();
    CryptoProvider::install_default(provider).expect("Failed to install default crypto provider");

    // load the default trust store if provided
    let mut root_store = rustls::RootCertStore::empty();
    for cert in load_native_certs().certs
    {
        root_store
            .add(cert)
            .context("failed to add native certificate to root store")?;
    }

    if let Some(trust_store_path) = config.certificate_trust_store
    {
        let cert_path = std::path::Path::new(&trust_store_path);
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
        for cert in cert_chain
        {
            root_store
                .add(cert)
                .context("failed to add certificate to root store")?;
        }
    }

    // Create client endpoint with the given server address and tls configuration
    let client_config = ClientConfig::with_root_certificates(Arc::new(root_store))?;
    let socket = SocketAddr::from(([0, 0, 0, 0], 0));
    let client_endpoint = Endpoint::client(socket).unwrap();

    // Connect to the server
    let connection = client_endpoint
        .connect_with(client_config, config.server_address, config.server_name.as_str())
        .context("failed to connect to server")?
        .await
        .context("failed to establish connection to server")?;

    println!("Connected to server at {}", config.server_address);

    // Create longboy client sessions
    let client_session = ClientSession::new(connection).await?;

    // Bind the cancellation token to sigterm handler
    let cancellation_token = CancellationToken::new();

    // Session established, create the longboy client using tokyo runtime.
    let runtime = longboy::TokioRuntime::new(cancellation_token.child_token());
    let client_to_server_schema = new_client_to_server_schema();
    let server_to_client_schema = new_server_to_client_schema();
    let receiver_channel = flume::unbounded();
    let sender_channel = flume::unbounded();
    let _longboy_client = Client::builder(client_session, Box::new(runtime))
        .receiver::<_, 32, 3>(
            &server_to_client_schema,
            ServerToClientSink {
                channel: receiver_channel.0,
            },
        )?
        .sender::<_, 16, 3>(
            &client_to_server_schema,
            ClientToServerSource {
                channel: sender_channel.1,
            },
        )?
        .build();

    // Client was created, for now just log and exit.
    println!("Longboy client session established.");

    // dump server events
    tokio::spawn(async move {
        let recv = receiver_channel.1.clone();
        loop
        {
            let incoming = recv.recv_async().await;

            match incoming
            {
                Ok((frame, player_id, player_input)) =>
                {
                    println!("Recv'd Frame({}) PlayerID({}) PlayerInput({})", frame, player_id, player_input);
                }
                Err(e) => 
                {
                    println!("{}", e);
                    break
                }
            }
        }
    });

    let mut frame: u32 = 0;
    loop
    {
        if cancellation_token.is_cancelled()
        {
            break;
        }
        // send keys
        sleep(Duration::from_millis(100)).await;
        // random ascii between 32 and 126
        let player_input: u64 = rand::random::<u8>() as u64 % (126 - 32) + 32;
        println!("Sending Frame({}) PlayerInput({})", frame, player_input);
        sender_channel.0.send((frame, player_input)).unwrap();
        frame += 1;
        println!("Frame {}", frame);
    }

    Ok(())
}
