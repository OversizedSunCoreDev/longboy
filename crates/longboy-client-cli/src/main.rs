use std::{net::SocketAddr, sync::Arc};

use config::Config;
use anyhow::{Context, Result};
use quinn::{Endpoint, ClientConfig};
use rustls::{crypto::{CryptoProvider, aws_lc_rs}, pki_types::{CertificateDer, pem::PemObject}};
use rustls_native_certs::load_native_certs;
use longboy::{Client, ClientSession};

#[derive(serde::Deserialize)]
struct LongboyClientConfig
{
    certificate_trust_store: Option<String>,
    server_address: SocketAddr,
    server_name: String,
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
        root_store.add(cert).context("failed to add native certificate to root store")?;
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
            root_store.add(cert).context("failed to add certificate to root store")?;
        }
    }

    // Create client endpoint with the given server address and tls configuration
    let client_config = ClientConfig::with_root_certificates(Arc::new(root_store))?;
    let client_endpoint = Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();

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
    let _longboy_client = Client::builder(client_session, Box::new(runtime)).build();

    // Client was created, for now just log and exit.
    println!("Longboy client session established.");

    Ok(())
}
