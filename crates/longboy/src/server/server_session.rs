use anyhow::Result;
use quinn::Connection;
use tokio::io::AsyncWriteExt;

pub struct ServerSession
{
    connection: Connection,
    session_id: u64,
    cipher_key: u64,
}

impl ServerSession
{
    pub async fn new(session_id: u64, cipher_key: u64, connection: Connection) -> Result<Self>
    {
        let mut send = connection.open_uni().await?;
        send.write_u64_le(session_id).await?;
        send.write_u64_le(cipher_key).await?;
        send.finish()?;
        send.stopped().await?;

        Ok(Self {
            connection: connection,
            session_id: session_id,
            cipher_key: cipher_key,
        })
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
