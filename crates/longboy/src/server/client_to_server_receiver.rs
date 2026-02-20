use std::net::{SocketAddr, UdpSocket};

use anyhow::Result;
use enum_map::{Enum, EnumMap};
use flume::Receiver as FlumeReceiver;
use fnv::FnvHashMap;
use thunderdome::{Arena, Index};
use tracing::{info, warn};

use crate::{Constants, Factory, Mirroring, Receiver, RuntimeTask, ServerSessionEvent, Sink};

pub(crate) struct ClientToServerReceiver<SinkFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>
where
    SinkFactoryType: Factory<Type: Sink<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    name: String,

    mapper_socket: UdpSocket,

    socket: UdpSocket,

    session_receiver: FlumeReceiver<ServerSessionEvent>,
    sessions: Arena<ReceiverSession<SinkFactoryType::Type, SIZE, WINDOW_SIZE>>,
    session_id_to_session_map: FnvHashMap<u64, Index>,
    socket_addr_to_session_map: FnvHashMap<SocketAddr, Index>,
    sink_factory: SinkFactoryType,
}

struct ReceiverSession<SinkType, const SIZE: usize, const WINDOW_SIZE: usize>
where
    SinkType: Sink<SIZE>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    socket_addrs: EnumMap<Mirroring, Option<SocketAddr>>,
    receiver: Receiver<SinkType, SIZE, WINDOW_SIZE>,
}

impl<SinkFactoryType, const SIZE: usize, const WINDOW_SIZE: usize> std::fmt::Debug
    for ClientToServerReceiver<SinkFactoryType, SIZE, WINDOW_SIZE>
where
    SinkFactoryType: Factory<Type: Sink<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        f.debug_struct("ClientToServerReceiver")
            .field("name", &self.name)
            .field("mapper_socket", &self.mapper_socket)
            .field("socket", &self.socket)
            .field("session_receiver", &"FlumeReceiver<ServerSessionEvent>")
            .field(
                "sessions",
                &format!(
                    "Arena<ReceiverSession<{}, {}, {}>>",
                    "SinkFactoryType::Type", SIZE, WINDOW_SIZE
                ),
            )
            .field("session_id_to_session_map", &self.session_id_to_session_map)
            .field("socket_addr_to_session_map", &self.socket_addr_to_session_map)
            .finish()
    }
}

impl<SinkFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>
    ClientToServerReceiver<SinkFactoryType, SIZE, WINDOW_SIZE>
where
    SinkFactoryType: Factory<Type: Sink<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    pub(crate) fn new(
        name: String,
        mapper_socket: UdpSocket,
        socket: UdpSocket,
        session_capacity: usize,
        session_receiver: FlumeReceiver<ServerSessionEvent>,
        sink_factory: SinkFactoryType,
    ) -> Result<Self>
    {
        mapper_socket.set_nonblocking(true)?;

        socket.set_nonblocking(true)?;

        Ok(Self {
            name,

            mapper_socket,

            socket,

            session_receiver,
            sessions: Arena::with_capacity(session_capacity),
            session_id_to_session_map: FnvHashMap::with_capacity_and_hasher(session_capacity, Default::default()),
            socket_addr_to_session_map: FnvHashMap::with_capacity_and_hasher(
                session_capacity * Mirroring::LENGTH,
                Default::default(),
            ),
            sink_factory,
        })
    }

    fn poll_server_session_events(&mut self, _timestamp: u16)
    {
        // Handle Session changes.
        for event in self.session_receiver.try_iter()
        {
            match event
            {
                ServerSessionEvent::Connected { session_id, cipher_key } =>
                {
                    let index = self.sessions.insert(ReceiverSession {
                        socket_addrs: EnumMap::default(),
                        receiver: Receiver::new(cipher_key, self.sink_factory.invoke(session_id)),
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
                    let session = self.sessions.remove(index).unwrap();
                    for socket_addr in session.socket_addrs.values().flatten()
                    {
                        self.socket_addr_to_session_map.remove(socket_addr);
                    }
                }
            }
        }
    }

    fn poll_for_client_socket_addresses(&mut self)
    {
        // Update Client socket addresses.
        let mut buffer = [0; 64];
        // TODO: This accepts anything, even those which are not mapped, no session is established, and there is no signed header.
        while let Ok((len, socket_addr)) = self.mapper_socket.recv_from(&mut buffer)
        {
            if len < std::mem::size_of::<u64>() + std::mem::size_of::<u8>() // disqualify not enough data?
            {
                warn!(
                    "Received mapping of invalid length {} from socket_addr {:?}",
                    len, socket_addr
                );
                continue;
            }

            let session_id = u64::from_le_bytes(*<&[u8; 8]>::try_from(&buffer[0..8]).unwrap());
            info!(
                "Received mapping from session_id {} to socket_addr {:?}",
                session_id, socket_addr
            );
            let mirroring = buffer[8] as usize;

            if mirroring >= Mirroring::LENGTH
            {
                continue;
            }
            let mirroring = Mirroring::from_usize(mirroring);

            if let Some(index) = self.session_id_to_session_map.get(&session_id)
            {
                let session = self.sessions.get_mut(*index).unwrap();

                if let Some(socket_addr) = session.socket_addrs[mirroring]
                {
                    info!("Removing old mapping from session_id {} to socket_addr {:?} for mirroring {:?}", session_id, socket_addr, mirroring);
                    self.socket_addr_to_session_map.remove(&socket_addr);
                }

                info!("Adding mapping from session_id {} to socket_addr {:?} for mirroring {:?}", session_id, socket_addr, mirroring);
                session.socket_addrs[mirroring] = Some(socket_addr);
                self.socket_addr_to_session_map.insert(socket_addr, *index);
            }
        }
    }

    fn poll_for_client_datagrams(&mut self, timestamp: u16)
    {
        // Alias constants so they're less painful to read.
        #[allow(non_snake_case)]
        let DATAGRAM_SIZE: usize = Constants::<SIZE, WINDOW_SIZE>::DATAGRAM_SIZE;

        // TODO: This accepts anything, even those which are not mapped, no session is established, and there is no signed header.
        let mut buffer = [0; 512];
        while let Ok((len, socket_addr)) = self.socket.recv_from(&mut buffer)
        {
            if len != DATAGRAM_SIZE
            {
                warn!(
                    "Received datagram of invalid length {} from socket_addr {:?}",
                    len, socket_addr
                );
                continue;
            }
            let datagram = (&mut buffer[0..DATAGRAM_SIZE]).try_into().unwrap();
            info!("Received datagram from socket_addr {:?} - datagram {:?}", socket_addr, datagram);

            if let Some(index) = self.socket_addr_to_session_map.get(&socket_addr)
            {
                self.sessions
                    .get_mut(*index)
                    .unwrap()
                    .receiver
                    .handle_datagram(timestamp, datagram);
            }
        }
    }
}

impl<SinkFactoryType, const SIZE: usize, const WINDOW_SIZE: usize> RuntimeTask
    for ClientToServerReceiver<SinkFactoryType, SIZE, WINDOW_SIZE>
where
    SinkFactoryType: Factory<Type: Sink<SIZE>>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    fn name(&self) -> &str
    {
        &self.name
    }

    fn poll(&mut self, timestamp: u16)
    {
        self.poll_server_session_events(timestamp);
        self.poll_for_client_socket_addresses();
        self.poll_for_client_datagrams(timestamp);
    }
}
