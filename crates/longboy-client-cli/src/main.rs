#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(generic_const_items)]
#![feature(unboxed_closures)]

use std::{
    net::{SocketAddr, UdpSocket},
    process,
    sync::Arc,
    thread::{self},
    time::Duration,
};

use anyhow::{Context, Result};
use config::Config;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use longboy::{Client, ClientSession, ClientToServerSchema, ServerToClientSchema, Sink, Source};
use quinn::{ClientConfig, Endpoint};
use rustls::{
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, pem::PemObject},
};
use rustls_native_certs::load_native_certs;

#[derive(serde::Deserialize)]
struct LongboyClientConfig
{
    certificate_trust_store: Option<String>,
    server_address: SocketAddr,
    server_name: String,
}

struct ServerToClientSink
{
    channel: flume::Sender<(u32, [u64; 2])>,
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
        let player_input_1 = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[4..12]).unwrap()));
        let player_input_2 = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[12..20]).unwrap()));
        self.channel.send((frame, [player_input_1, player_input_2])).unwrap();
    }
}

fn main()
{
    let _ = enable_raw_mode(); // Enters raw mode

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

    let _ = disable_raw_mode(); // Exits raw mode
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

    // Session established, create the longboy client using tokyo runtime.
    let runtime = longboy::TokioRuntime::new(tokio_util::sync::CancellationToken::new());
    let client_to_server_schema = ClientToServerSchema {
        name: "Input",
        mapper_port: 8081,
        heartbeat_period: 2000,
        port: 8082,
    };

    let server_to_client_schema = ServerToClientSchema {
        name: "State",
        mapper_port: 8080,
        heartbeat_period: 2000,
    };

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
                Ok((frame, [first, second])) =>
                {
                    println!("Recv'd Frame({}) ({} ; {})", frame, first, second)
                }
                Err(e) => println!("{}", e),
            }
        }
    });

    let mut frame: u32 = 0;
    // init with some garbage
    sender_channel.0.send((frame, u64::from('z')))?;
    loop
    {
        // send keys
        if let Event::Key(key_event) = event::read().unwrap()
        {
            match key_event.code
            {
                KeyCode::Esc => process::exit(0),
                KeyCode::Char(val) =>
                {
                    println!("Sending Frame({}) {}", frame, val);
                    let as64 = u64::from(val);
                    sender_channel.0.send((frame, as64)).unwrap();
                }
                _ => (),
            }
        }
        frame += 1;
        thread::sleep(Duration::from_millis(1));
    }
}
