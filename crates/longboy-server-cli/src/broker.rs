use flume::{Receiver, Sender};
use longboy::{Factory, Sink, Source};
use std::{collections::HashMap, sync::{Arc, RwLock, atomic::{AtomicU64}}};
use anyhow::{Result};
use std::sync::atomic::Ordering::SeqCst;

pub struct SessionBroker<const MAX_PLAYERS: usize>
{
    receiver_channels: [Receiver<(u32, u8, u64)>; MAX_PLAYERS],
    broadcast_channel: Sender<(u32, u8, u64)>,
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
        let receiver_channels: [Receiver<(u32, u8, u64)>; MAX_PLAYERS] =
            [(); MAX_PLAYERS].map(|_| {
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
            Some(old_player_index) => println!("Warning: Overwriting existing session {} for player index {} with {}", old_player_index, player_index, session_id),
            None => println!("Allocated session id {} for player index {}", session_id, player_index),
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

    // pub fn free_player_index(&self, player_index: usize)
    // {
    //     let mut free_list = self.player_index_free_list.write().unwrap();
    //     free_list[player_index] = true;
    //     let mut sessions = self.player_sessions.write().unwrap();
    //     let session_id = sessions.iter().find_map(|(&s_id, &p_index)| if p_index == player_index { Some(s_id) } else { None });
    //     if let Some(s_id) = session_id
    //     {
    //         sessions.remove(&s_id);
    //         println!("Freed session id {} for player index {}", s_id, player_index);
    //     }
    // }
}


pub struct ClientToServerSink
{
    player_index: u8,
    channel: Sender<(u32, u8, u64)>,
}

pub struct ServerToClientSource
{
    channel: Receiver<(u32, u8, u64)>,
}

pub struct ServerBroker<const MAX_PLAYERS: usize>
{
    inner: Arc<SessionBroker<MAX_PLAYERS>>,
}

impl<const MAX_PLAYERS: usize> ServerBroker<MAX_PLAYERS>
{
    pub fn new(session_broker: Arc<SessionBroker<MAX_PLAYERS>>) -> Self
    {
        Self {
            inner: session_broker,
        }
    }
}

impl<const MAX_PLAYERS: usize> Factory for ServerBroker<MAX_PLAYERS>
{
    type Type = ServerToClientSource;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let binding = self
            .inner
            .player_sessions
            .read()
            .unwrap();
        let player_index = binding
            .get(&session_id)
            .expect("Unknown Session ID");

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
        Self {
            inner: session_broker,
        }
    }
}

impl<const MAX_PLAYERS: usize> Factory for ClientBroker<MAX_PLAYERS>
{
    type Type = ClientToServerSink;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let binding = self
            .inner
            .player_sessions
            .read()
            .unwrap();
        let player_index = binding
            .get(&session_id)
            .expect("Unknown Session ID");

        ClientToServerSink {
            player_index: *player_index as u8,
            channel: self.inner.broadcast_channel.clone(),
        }
    }
}

impl Sink<16> for ClientToServerSink
{
    fn handle(&mut self, buffer: &[u8; 16])
    {
        let frame = u32::from_le_bytes(*(<&[u8; 4]>::try_from(&buffer[0..4]).unwrap()));
        let player_input = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[4..12]).unwrap()));
        println!("Recv'd Frame({}) PlayerInput({})", frame, player_input);
        self.channel.send((frame, self.player_index, player_input)).unwrap();
    }
}

impl Source<32> for ServerToClientSource
{
    fn poll(&mut self, buffer: &mut [u8; 32]) -> bool
    {
        match self.channel.try_recv()
        {
            Ok((frame, player_id, player_input)) =>
            {
                *(<&mut [u8; 4]>::try_from(&mut buffer[0..4]).unwrap()) = frame.to_le_bytes();
                *(<&mut [u8; 1]>::try_from(&mut buffer[4..5]).unwrap()) = player_id.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[5..13]).unwrap()) = player_input.to_le_bytes();
                println!("Sending Frame({}) PlayerID({}) PlayerInput({})", frame, player_id, player_input);
                true
            }
            Err(_) => false,
        }
    }
}