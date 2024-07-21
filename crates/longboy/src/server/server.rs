use std::net::{SocketAddr, UdpSocket};

use anyhow::{anyhow, Context, Result};
use enum_map::{enum_map, EnumMap};
use flume::Sender as FlumeSender;
use fnv::{FnvHashMap, FnvHashSet};

use crate::{
    ClientToServerReceiver, ClientToServerSchema, Constants, Factory, Mirroring, Runtime, RuntimeTask, ServerSession,
    ServerSessionEvent, ServerToClientSchema, ServerToClientSender, Sink, Source,
};

pub struct Server
{
    sessions: FnvHashMap<u64, ServerSession>,
    session_senders: Box<[FlumeSender<ServerSessionEvent>]>,
    runtime: Box<dyn Runtime>,
}

pub struct ServerBuilder
{
    session_capacity: usize,
    runtime: Box<dyn Runtime>,

    ports: FnvHashSet<u16>,
    session_senders: Vec<FlumeSender<ServerSessionEvent>>,
    tasks: Vec<Box<dyn RuntimeTask>>,
}

impl Server
{
    pub fn builder(session_capacity: usize, runtime: Box<dyn Runtime>) -> ServerBuilder
    {
        ServerBuilder {
            session_capacity: session_capacity,
            runtime: runtime,

            ports: FnvHashSet::default(),
            tasks: Vec::new(),
            session_senders: Vec::new(),
        }
    }

    pub fn register(&mut self, session: ServerSession)
    {
        assert!(self.sessions.len() < self.sessions.capacity());

        let session_id = session.session_id();
        let cipher_key = session.cipher_key();

        self.sessions.insert(session_id, session);
        self.session_senders.iter().for_each(|session_sender| {
            session_sender
                .send(ServerSessionEvent::Connected {
                    session_id: session_id,
                    cipher_key: cipher_key,
                })
                .unwrap()
        });
    }

    pub fn unregister(&mut self, session_id: u64)
    {
        assert!(self.sessions.contains_key(&session_id));

        self.session_senders.iter().for_each(|session_sender| {
            session_sender
                .send(ServerSessionEvent::Disconnected { session_id: session_id })
                .unwrap()
        });
        self.sessions.remove(&session_id);
    }
}

impl ServerBuilder
{
    pub fn sender<SourceFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>(
        self,
        schema: &ServerToClientSchema,
        source_factory: SourceFactoryType,
    ) -> Result<Self>
    where
        SourceFactoryType: Factory<Type: Source<SIZE>>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    {
        let mapper_socket =
            UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], schema.mapper_port))).context(schema.name)?;

        let sockets = enum_map! {
            Mirroring::AudioVideo => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?,
            Mirroring::Background => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?,
            Mirroring::Voice => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?,
        };

        self.sender_with_sockets::<SourceFactoryType, SIZE, WINDOW_SIZE>(schema, mapper_socket, sockets, source_factory)
    }

    pub fn sender_with_sockets<SourceFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>(
        mut self,
        schema: &ServerToClientSchema,
        mapper_socket: UdpSocket,
        sockets: EnumMap<Mirroring, UdpSocket>,
        source_factory: SourceFactoryType,
    ) -> Result<Self>
    where
        SourceFactoryType: Factory<Type: Source<SIZE>>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    {
        if schema.mapper_port != mapper_socket.local_addr().unwrap().port()
        {
            return Err(anyhow!(
                "Schema's `mapper_port` does not match Mapper Socket port: {} vs {}",
                schema.mapper_port,
                mapper_socket.local_addr().unwrap().port()
            ))
            .context(schema.name);
        }
        if !self.ports.insert(schema.mapper_port)
        {
            return Err(anyhow!("Reused port {}", schema.mapper_port)).context(schema.name);
        }

        let (session_sender, session_receiver) = flume::unbounded();

        let server_to_client_sender = ServerToClientSender::<SourceFactoryType, SIZE, WINDOW_SIZE>::new(
            format!("ServerToClientSender: {}", schema.name),
            mapper_socket,
            sockets,
            self.session_capacity,
            session_receiver,
            source_factory,
        )
        .context(schema.name)?;

        self.tasks.push(Box::new(server_to_client_sender));
        self.session_senders.push(session_sender);
        Ok(self)
    }

    pub fn receiver<SinkFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>(
        self,
        schema: &ClientToServerSchema,
        sink_factory: SinkFactoryType,
    ) -> Result<Self>
    where
        SinkFactoryType: Factory<Type: Sink<SIZE>>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
        [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
    {
        let mapper_socket =
            UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], schema.mapper_port))).context(schema.name)?;

        let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?;

        self.receiver_with_socket::<SinkFactoryType, SIZE, WINDOW_SIZE>(schema, mapper_socket, socket, sink_factory)
    }

    pub fn receiver_with_socket<SinkFactoryType, const SIZE: usize, const WINDOW_SIZE: usize>(
        mut self,
        schema: &ClientToServerSchema,
        mapper_socket: UdpSocket,
        socket: UdpSocket,
        sink_factory: SinkFactoryType,
    ) -> Result<Self>
    where
        SinkFactoryType: Factory<Type: Sink<SIZE>>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
        [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
    {
        if schema.mapper_port != mapper_socket.local_addr().unwrap().port()
        {
            return Err(anyhow!(
                "Schema's `mapper_port` does not match Mapper Socket port: {} vs {}",
                schema.mapper_port,
                mapper_socket.local_addr().unwrap().port()
            ))
            .context(schema.name);
        }
        if schema.port != socket.local_addr().unwrap().port()
        {
            return Err(anyhow!(
                "Schema's `port` does not match Socket port: {} vs {}",
                schema.mapper_port,
                mapper_socket.local_addr().unwrap().port()
            ))
            .context(schema.name);
        }
        if !self.ports.insert(schema.mapper_port)
        {
            return Err(anyhow!("Reused port {}", schema.mapper_port)).context(schema.name);
        }

        socket.set_nonblocking(true).context(schema.name)?;

        let (session_sender, session_receiver) = flume::unbounded();

        let client_to_server_receiver = ClientToServerReceiver::<SinkFactoryType, SIZE, WINDOW_SIZE>::new(
            format!("ClientToServerReceiver: {}", schema.name),
            mapper_socket,
            socket,
            self.session_capacity,
            session_receiver,
            sink_factory,
        )
        .context(schema.name)?;

        self.tasks.push(Box::new(client_to_server_receiver));
        self.session_senders.push(session_sender);
        Ok(self)
    }

    pub fn build(mut self) -> Server
    {
        for task in self.tasks.into_iter()
        {
            self.runtime.spawn(task);
        }

        Server {
            sessions: FnvHashMap::with_capacity_and_hasher(self.session_capacity, Default::default()),
            session_senders: self.session_senders.into_boxed_slice(),
            runtime: self.runtime,
        }
    }
}
