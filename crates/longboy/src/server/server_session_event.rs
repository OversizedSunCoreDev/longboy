pub(crate) enum ServerSessionEvent
{
    Connected
    {
        session_id: u64, cipher_key: u64
    },
    Disconnected
    {
        session_id: u64
    },
}
