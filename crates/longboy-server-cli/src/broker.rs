use anyhow::Result;
use flume::{Receiver, Sender};
use longboy::{Factory, Sink, Source};
use tracing::info;
use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering::SeqCst},
    },
};

pub struct SessionBroker<const MAX_PLAYERS: usize>
{
    receiver_channels: [Receiver<(u64, u64, u64, u8)>; MAX_PLAYERS],
    broadcast_channel: Sender<(u64, u64, u64, u8)>,
    // thread safe atomic always incrementing session id
    next_session_id: AtomicU64,
    // key is the session id, value is the player index
    player_sessions: Arc<RwLock<HashMap<u64, usize>>>,
    // thread safe atomic free pointer to the next player index
    player_index_free_list: Arc<RwLock<Vec<bool>>>,
}

impl<const MAX_PLAYERS: usize> SessionBroker<MAX_PLAYERS>
{
    pub fn new() -> Self
    {
        let pipe = flume::unbounded();
        let receiver_channels: [Receiver<(u64, u64, u64, u8)>; MAX_PLAYERS] = [(); MAX_PLAYERS].map(|_| {
            let (_s, r) = pipe.clone();
            r
        });

        let broadcast_channel = pipe.0;

        Self {
            receiver_channels,
            broadcast_channel,
            next_session_id: AtomicU64::new(1),
            player_sessions: Arc::new(RwLock::new(HashMap::new())),
            player_index_free_list: Arc::new(RwLock::new((0..MAX_PLAYERS).map(|_| true).collect())),
        }
    }

    pub fn allocate_session_id(&self, player_index: usize) -> u64
    {
        let session_id = self.next_session_id.fetch_add(1, SeqCst);
        let insert = self.player_sessions.write().unwrap().insert(session_id, player_index);
        match insert
        {
            Some(old_player_index) => info!(
                "Warning: Overwriting existing session {} for player index {} with {}",
                old_player_index, player_index, session_id
            ),
            None => info!("Allocated session id {} for player index {}", session_id, player_index),
        }
        session_id
    }

    pub fn next_player_index(&self) -> Result<usize>
    {
        let mut free_list = self.player_index_free_list.write().unwrap();
        let player_index = free_list.iter().position(|&is_free| is_free);
        match player_index
        {
            Some(index) =>
            {
                free_list[index] = false;
                Ok(index)
            }
            None => anyhow::bail!("Maximum number of players reached"),
        }
    }
}

pub struct ClientToServerSink
{
    player_index: u8,
    channel: Sender<(u64, u64, u64, u8)>,
}

pub struct ServerToClientSource
{
    channel: Receiver<(u64, u64, u64, u8)>,
}

pub struct ServerBroker<const MAX_PLAYERS: usize>
{
    inner: Arc<SessionBroker<MAX_PLAYERS>>,
}

impl<const MAX_PLAYERS: usize> ServerBroker<MAX_PLAYERS>
{
    pub fn new(session_broker: Arc<SessionBroker<MAX_PLAYERS>>) -> Self
    {
        Self { inner: session_broker }
    }
}

impl<const MAX_PLAYERS: usize> Factory for ServerBroker<MAX_PLAYERS>
{
    type Type = ServerToClientSource;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let binding = self.inner.player_sessions.read().unwrap();
        let player_index = binding.get(&session_id).expect("Unknown Session ID");

        ServerToClientSource {
            channel: self.inner.receiver_channels[*player_index].clone(),
        }
    }
}

pub struct ClientBroker<const MAX_PLAYERS: usize>
{
    inner: Arc<SessionBroker<MAX_PLAYERS>>,
}

impl<const MAX_PLAYERS: usize> ClientBroker<MAX_PLAYERS>
{
    pub fn new(session_broker: Arc<SessionBroker<MAX_PLAYERS>>) -> Self
    {
        Self { inner: session_broker }
    }
}

impl<const MAX_PLAYERS: usize> Factory for ClientBroker<MAX_PLAYERS>
{
    type Type = ClientToServerSink;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let binding = self.inner.player_sessions.read().unwrap();
        let player_index = binding.get(&session_id).expect("Unknown Session ID");

        ClientToServerSink {
            player_index: *player_index as u8,
            channel: self.inner.broadcast_channel.clone(),
        }
    }
}

impl Sink<28> for ClientToServerSink
{
    #[tracing::instrument(skip(self))]
    fn handle(&mut self, buffer: &[u8; 28])
    {
        let a = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[0..8]).unwrap()));
        let b = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[8..16]).unwrap()));
        let c = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[16..24]).unwrap()));
        self.channel.send((a, b, c, self.player_index)).unwrap();
    }
}

impl Source<28> for ServerToClientSource
{
    #[tracing::instrument(skip(self))]
    fn poll(&mut self, buffer: &mut [u8; 28]) -> bool
    {
        match self.channel.try_recv()
        {
            Ok((a, b, c, player_index)) =>
            {
                *(<&mut [u8; 8]>::try_from(&mut buffer[0..8]).unwrap()) = a.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[8..16]).unwrap()) = b.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[16..24]).unwrap()) = c.to_le_bytes();
                *(<&mut [u8; 1]>::try_from(&mut buffer[24..25]).unwrap()) = player_index.to_le_bytes();
                true
            }
            Err(_) => false,
        }
    }
}
