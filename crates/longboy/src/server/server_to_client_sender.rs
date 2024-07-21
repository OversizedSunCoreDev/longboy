use std::net::{SocketAddr, UdpSocket};

use anyhow::Result;
use enum_map::EnumMap;
use flume::Receiver as FlumeReceiver;
use fnv::FnvHashMap;
use thunderdome::{Arena, Index};

use crate::{Constants, Factory, Mirroring, RuntimeTask, Sender, ServerSessionEvent, Source, UdpSocketExt};

pub(crate) struct ServerToClientSender<SourceFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>
where
    SourceFactoryType: Factory<Type: Source<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    name: String,

    mapper_socket: UdpSocket,

    sockets: EnumMap<Mirroring, UdpSocket>,

    session_receiver: FlumeReceiver<ServerSessionEvent>,
    sessions: Arena<SenderSession<SourceFactoryType::Type, SIZE, WINDOW_SIZE>>,
    session_id_to_session_map: FnvHashMap<u64, Index>,
    source_factory: SourceFactoryType,
}

struct SenderSession<SourceType, const SIZE: usize, const WINDOW_SIZE: usize>
where
    SourceType: Source<SIZE>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    socket_addr: Option<SocketAddr>,
    sender: Sender<SourceType, SIZE, WINDOW_SIZE>,
}

impl<SourceFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>
    ServerToClientSender<SourceFactoryType, SIZE, WINDOW_SIZE>
where
    SourceFactoryType: Factory<Type: Source<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    pub(crate) fn new(
        name: String,
        mapper_socket: UdpSocket,
        sockets: EnumMap<Mirroring, UdpSocket>,
        session_capacity: usize,
        session_receiver: FlumeReceiver<ServerSessionEvent>,
        source_factory: SourceFactoryType,
    ) -> Result<Self>
    {
        mapper_socket.set_nonblocking(true)?;

        sockets[Mirroring::AudioVideo].set_nonblocking(true)?;
        sockets[Mirroring::AudioVideo].set_qos_audio_video()?;

        sockets[Mirroring::Background].set_nonblocking(true)?;
        sockets[Mirroring::Background].set_qos_background()?;

        sockets[Mirroring::Voice].set_nonblocking(true)?;
        sockets[Mirroring::Voice].set_qos_voice()?;

        Ok(Self {
            name: name,

            mapper_socket: mapper_socket,

            sockets: sockets,

            session_receiver: session_receiver,
            sessions: Arena::with_capacity(session_capacity),
            session_id_to_session_map: FnvHashMap::with_capacity_and_hasher(session_capacity, Default::default()),
            source_factory: source_factory,
        })
    }
}

impl<SourceFactoryType, const SIZE: usize, const WINDOW_SIZE: usize> RuntimeTask
    for ServerToClientSender<SourceFactoryType, SIZE, WINDOW_SIZE>
where
    SourceFactoryType: Factory<Type: Source<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    fn name(&self) -> &str
    {
        &self.name
    }

    fn poll(&mut self, timestamp: u16)
    {
        // Handle Session changes.
        for event in self.session_receiver.try_iter()
        {
            match event
            {
                ServerSessionEvent::Connected { session_id, cipher_key } =>
                {
                    let index = self.sessions.insert(SenderSession {
                        socket_addr: None,
                        sender: Sender::new(cipher_key, self.source_factory.invoke(session_id)),
                    });
                    self.session_id_to_session_map
                        .try_insert(session_id, index)
                        .expect("Duplicate Session ID");
                }
                ServerSessionEvent::Disconnected { session_id } =>
                {
                    let index = self
                        .session_id_to_session_map
                        .remove(&session_id)
                        .expect("Unknown Session ID");
                    self.sessions.remove(index);
                }
            }
        }

        // Update Client socket addresses.
        let mut buffer = [0; 64];
        while let Ok((len, socket_addr)) = self.mapper_socket.recv_from(&mut buffer)
        {
            if len != std::mem::size_of::<u64>()
            {
                continue;
            }

            let session_id = u64::from_le_bytes(*<&[u8; 8]>::try_from(&buffer[0..8]).unwrap());

            if let Some(index) = self.session_id_to_session_map.get(&session_id)
            {
                self.sessions[*index].socket_addr = Some(socket_addr);
            }
        }

        // Poll Sessions
        for (_, session) in self.sessions.iter_mut()
        {
            if let Some(datagram) = session.sender.poll_datagram(timestamp)
                && let Some(socket_addr) = session.socket_addr
            {
                for socket in self.sockets.values()
                {
                    socket.send_to(datagram, socket_addr).expect("send_to failure");
                }
            }
        }
    }
}
