use std::net::{SocketAddr, UdpSocket};

use anyhow::Result;
use enum_map::{Enum, EnumMap};
use flume::Receiver as FlumeReceiver;
use fnv::FnvHashMap;
use thunderdome::{Arena, Index};

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
            name: name,

            mapper_socket: mapper_socket,

            socket: socket,

            session_receiver: session_receiver,
            sessions: Arena::with_capacity(session_capacity),
            session_id_to_session_map: FnvHashMap::with_capacity_and_hasher(session_capacity, Default::default()),
            socket_addr_to_session_map: FnvHashMap::with_capacity_and_hasher(
                session_capacity * Mirroring::LENGTH,
                Default::default(),
            ),
            sink_factory: sink_factory,
        })
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
        // Alias constants so they're less painful to read.
        #[allow(non_snake_case)]
        let DATAGRAM_SIZE: usize = Constants::<SIZE, WINDOW_SIZE>::DATAGRAM_SIZE;

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

        // Update Client socket addresses.
        let mut buffer = [0; 64];
        while let Ok((len, socket_addr)) = self.mapper_socket.recv_from(&mut buffer)
        {
            if len != std::mem::size_of::<u64>() + std::mem::size_of::<u8>()
            {
                continue;
            }

            let session_id = u64::from_le_bytes(*<&[u8; 8]>::try_from(&buffer[0..8]).unwrap());

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
                    self.socket_addr_to_session_map.remove(&socket_addr);
                }

                session.socket_addrs[mirroring] = Some(socket_addr);
                self.socket_addr_to_session_map.insert(socket_addr, *index);
            }
        }

        // Process datagrams.
        let mut buffer = [0; 512];
        while let Ok((len, socket_addr)) = self.socket.recv_from(&mut buffer)
        {
            if len != DATAGRAM_SIZE
            {
                continue;
            }
            let datagram = (&mut buffer[0..DATAGRAM_SIZE]).try_into().unwrap();

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
