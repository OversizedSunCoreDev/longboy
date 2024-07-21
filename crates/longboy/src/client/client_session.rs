use std::net::IpAddr;

use anyhow::Result;
use quinn::Connection;
use tokio::io::AsyncReadExt;

pub struct ClientSession
{
    connection: Connection,
    session_id: u64,
    cipher_key: u64,
}

impl ClientSession
{
    pub async fn new(connection: Connection) -> Result<Self>
    {
        let mut receive = connection.accept_uni().await?;
        let session_id = receive.read_u64_le().await?;
        let cipher_key = receive.read_u64_le().await?;
        receive.stop(Default::default())?;

        Ok(Self {
            connection: connection,
            session_id: session_id,
            cipher_key: cipher_key,
        })
    }

    pub(crate) fn ip_addr(&self) -> IpAddr
    {
        self.connection.remote_address().ip()
    }

    pub(crate) fn session_id(&self) -> u64
    {
        self.session_id
    }

    pub(crate) fn cipher_key(&self) -> u64
    {
        self.cipher_key
    }
}
