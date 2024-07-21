use std::net::{SocketAddr, UdpSocket};

use anyhow::{anyhow, Context, Result};
use enum_map::{enum_map, EnumMap};
use fnv::FnvHashSet;

use crate::{
    ClientSession, ClientToServerSchema, ClientToServerSender, Constants, Mirroring, Runtime, RuntimeTask,
    ServerToClientReceiver, ServerToClientSchema, Sink, Source,
};

pub struct Client
{
    session: ClientSession,
    runtime: Box<dyn Runtime>,
}

pub struct ClientBuilder
{
    session: ClientSession,
    runtime: Box<dyn Runtime>,

    ports: FnvHashSet<u16>,
    tasks: Vec<Box<dyn RuntimeTask>>,
}

impl Client
{
    pub fn builder(session: ClientSession, runtime: Box<dyn Runtime>) -> ClientBuilder
    {
        ClientBuilder {
            session: session,
            runtime: runtime,

            ports: FnvHashSet::default(),
            tasks: Vec::new(),
        }
    }
}

impl ClientBuilder
{
    pub fn sender<SourceType, const SIZE: usize, const WINDOW_SIZE: usize>(
        self,
        schema: &ClientToServerSchema,
        source: SourceType,
    ) -> Result<Self>
    where
        SourceType: Source<SIZE>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    {
        let sockets = enum_map! {
            Mirroring::AudioVideo => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?,
            Mirroring::Background => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?,
            Mirroring::Voice => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?,
        };

        self.sender_with_sockets::<SourceType, SIZE, WINDOW_SIZE>(schema, sockets, source)
    }

    pub fn sender_with_sockets<SourceType, const SIZE: usize, const WINDOW_SIZE: usize>(
        mut self,
        schema: &ClientToServerSchema,
        sockets: EnumMap<Mirroring, UdpSocket>,
        source: SourceType,
    ) -> Result<Self>
    where
        SourceType: Source<SIZE>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    {
        if !self.ports.insert(schema.mapper_port)
        {
            return Err(anyhow!("Reused port {}", schema.mapper_port)).context(schema.name);
        }
        if !self.ports.insert(schema.port)
        {
            return Err(anyhow!("Reused port {}", schema.port)).context(schema.name);
        }

        let client_to_server_sender = ClientToServerSender::<SourceType, SIZE, WINDOW_SIZE>::new(
            format!("ClientToServerSender: {}", schema.name),
            SocketAddr::from((self.session.ip_addr(), schema.mapper_port)),
            schema.heartbeat_period,
            SocketAddr::from((self.session.ip_addr(), schema.port)),
            self.session.session_id(),
            self.session.cipher_key(),
            sockets,
            source,
        )
        .context(schema.name)?;

        self.tasks.push(Box::new(client_to_server_sender));
        Ok(self)
    }

    pub fn receiver<SinkType, const SIZE: usize, const WINDOW_SIZE: usize>(
        self,
        schema: &ServerToClientSchema,
        sink: SinkType,
    ) -> Result<Self>
    where
        SinkType: Sink<SIZE>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
        [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
    {
        let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).context(schema.name)?;

        self.receiver_with_socket::<SinkType, SIZE, WINDOW_SIZE>(schema, socket, sink)
    }

    pub fn receiver_with_socket<SinkType, const SIZE: usize, const WINDOW_SIZE: usize>(
        mut self,
        schema: &ServerToClientSchema,
        socket: UdpSocket,
        sink: SinkType,
    ) -> Result<Self>
    where
        SinkType: Sink<SIZE>,
        [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
        [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
    {
        if !self.ports.insert(schema.mapper_port)
        {
            return Err(anyhow!("Reused port {}", schema.mapper_port)).context(schema.name);
        }

        let server_to_client_receiver = ServerToClientReceiver::<SinkType, SIZE, WINDOW_SIZE>::new(
            format!("ServerToClientReceiver: {}", schema.name),
            SocketAddr::from((self.session.ip_addr(), schema.mapper_port)),
            schema.heartbeat_period,
            self.session.session_id(),
            self.session.cipher_key(),
            socket,
            sink,
        )
        .context(schema.name)?;

        self.tasks.push(Box::new(server_to_client_receiver));
        Ok(self)
    }

    pub fn build(mut self) -> Client
    {
        for task in self.tasks.into_iter()
        {
            self.runtime.spawn(task);
        }

        Client {
            runtime: self.runtime,
            session: self.session,
        }
    }
}
