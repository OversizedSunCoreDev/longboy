use std::{
    net::{SocketAddr, UdpSocket},
    sync::Arc,
};

use enum_map::enum_map;
use flume::{Receiver as FlumeReceiver, Sender as FlumeSender};
use parking_lot::Mutex;

use longboy::{
    Client, ClientSession, ClientToServerSchema, Factory, Mirroring, Runtime, RuntimeTask, Server, ServerSession,
    ServerToClientSchema, Sink, Source,
};
use quinn::{
    rustls::{
        pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
        RootCertStore,
    },
    ClientConfig, Connection, Endpoint, ServerConfig,
};
use rcgen::CertifiedKey;
use tokio::join;

#[derive(Clone)]
struct TestRuntime
{
    inner: Arc<Mutex<TestRuntimeInner>>,
}

struct TestRuntimeInner
{
    tick_period: u16,

    timestamp: u16,
    tasks: Vec<Box<dyn RuntimeTask>>,
}

struct TestServerToClientSourceFactory
{
    channels: [FlumeReceiver<(u32, [u64; 2])>; 2],
}

struct TestClientToServerSinkFactory
{
    channel: FlumeSender<(u32, u8, u64)>,
}

struct TestClientToServerSource
{
    channel: FlumeReceiver<(u32, u64)>,
}

struct TestClientToServerSink
{
    player_index: u8,
    channel: FlumeSender<(u32, u8, u64)>,
}

struct TestServerToClientSource
{
    channel: FlumeReceiver<(u32, [u64; 2])>,
}

struct TestServerToClientSink
{
    channel: FlumeSender<(u32, [u64; 2])>,
}

impl TestRuntime
{
    fn new(tick_period: u16) -> Self
    {
        Self {
            inner: Arc::new(Mutex::new(TestRuntimeInner {
                tick_period: tick_period,

                timestamp: 0,
                tasks: Vec::new(),
            })),
        }
    }

    fn tick(&self)
    {
        let mut inner = self.inner.lock();
        inner.timestamp += inner.tick_period;

        let timestamp = inner.timestamp;
        inner.tasks.iter_mut().for_each(|task| task.poll(timestamp));
    }
}

impl Runtime for TestRuntime
{
    fn running(&self) -> bool
    {
        true
    }

    fn spawn(&mut self, task: Box<dyn longboy::RuntimeTask>)
    {
        self.inner.lock().tasks.push(task);
    }
}

impl Factory for TestServerToClientSourceFactory
{
    type Type = TestServerToClientSource;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let player_index = match session_id
        {
            1 => 0,
            2 => 1,
            _ => unreachable!(),
        };

        TestServerToClientSource {
            channel: self.channels[player_index].clone(),
        }
    }
}

impl Factory for TestClientToServerSinkFactory
{
    type Type = TestClientToServerSink;

    fn invoke(&mut self, session_id: u64) -> Self::Type
    {
        let player_index = match session_id
        {
            1 => 0,
            2 => 1,
            _ => unreachable!(),
        };

        TestClientToServerSink {
            player_index: player_index,
            channel: self.channel.clone(),
        }
    }
}

impl Source<16> for TestClientToServerSource
{
    fn poll(&mut self, buffer: &mut [u8; 16]) -> bool
    {
        match self.channel.try_recv()
        {
            Ok((frame, player_input)) =>
            {
                *(<&mut [u8; 4]>::try_from(&mut buffer[0..4]).unwrap()) = frame.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[4..12]).unwrap()) = player_input.to_le_bytes();
                true
            }
            Err(_) => false,
        }
    }
}

impl Sink<16> for TestClientToServerSink
{
    fn handle(&mut self, buffer: &[u8; 16])
    {
        let frame = u32::from_le_bytes(*(<&[u8; 4]>::try_from(&buffer[0..4]).unwrap()));
        let player_input = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[4..12]).unwrap()));
        self.channel.send((frame, self.player_index, player_input)).unwrap();
    }
}

impl Source<32> for TestServerToClientSource
{
    fn poll(&mut self, buffer: &mut [u8; 32]) -> bool
    {
        match self.channel.try_recv()
        {
            Ok((frame, player_inputs)) =>
            {
                *(<&mut [u8; 4]>::try_from(&mut buffer[0..4]).unwrap()) = frame.to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[4..12]).unwrap()) = player_inputs[0].to_le_bytes();
                *(<&mut [u8; 8]>::try_from(&mut buffer[12..20]).unwrap()) = player_inputs[1].to_le_bytes();
                true
            }
            Err(_) => false,
        }
    }
}

impl Sink<32> for TestServerToClientSink
{
    fn handle(&mut self, buffer: &[u8; 32])
    {
        let frame = u32::from_le_bytes(*(<&[u8; 4]>::try_from(&buffer[0..4]).unwrap()));
        let player_input_1 = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[4..12]).unwrap()));
        let player_input_2 = u64::from_le_bytes(*(<&[u8; 8]>::try_from(&buffer[12..20]).unwrap()));
        self.channel.send((frame, [player_input_1, player_input_2])).unwrap();
    }
}

async fn connect(
    server_endpoint: &Endpoint,
    client_endpoint: &Endpoint,
    certified_key: &CertifiedKey,
) -> (Connection, Connection)
{
    let mut root_cert_store = RootCertStore::empty();
    root_cert_store.add(certified_key.cert.der().clone()).unwrap();

    join!(
        async { server_endpoint.accept().await.unwrap().await.unwrap() },
        async {
            client_endpoint
                .connect_with(
                    ClientConfig::with_root_certificates(Arc::new(root_cert_store)).unwrap(),
                    server_endpoint.local_addr().unwrap(),
                    "localhost",
                )
                .unwrap()
                .await
                .unwrap()
        }
    )
}

#[tokio::test]
async fn golden()
{
    const TICK_PERIOD: u16 = 0;

    let certified_key = rcgen::generate_simple_self_signed([String::from("localhost")]).unwrap();

    let server_endpoint = Endpoint::server(
        ServerConfig::with_single_cert(
            Vec::from([certified_key.cert.der().clone()]),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified_key.key_pair.serialize_der())),
        )
        .unwrap(),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    )
    .unwrap();
    let client_endpoint_1 = Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
    let client_endpoint_2 = Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();

    let connections_1 = connect(&server_endpoint, &client_endpoint_1, &certified_key).await;
    let connections_2 = connect(&server_endpoint, &client_endpoint_2, &certified_key).await;

    let server_session_1 = ServerSession::new(1, 0xDEADBEEFDEADBEEF, connections_1.0)
        .await
        .unwrap();
    let server_session_2 = ServerSession::new(2, 0xBEEFDEADBEEFDEAD, connections_2.0)
        .await
        .unwrap();
    let client_session_1 = ClientSession::new(connections_1.1).await.unwrap();
    let client_session_2 = ClientSession::new(connections_2.1).await.unwrap();

    let client_to_server_mapper_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();
    let client_to_server_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();

    let server_to_client_mapper_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();

    let client_to_server_schema = ClientToServerSchema {
        name: "Input",

        mapper_port: client_to_server_mapper_socket.local_addr().unwrap().port(),
        heartbeat_period: 2000,

        port: client_to_server_socket.local_addr().unwrap().port(),
    };

    let server_to_client_schema = ServerToClientSchema {
        name: "State",

        mapper_port: server_to_client_mapper_socket.local_addr().unwrap().port(),
        heartbeat_period: 2000,
    };

    let server_runtime = TestRuntime::new(TICK_PERIOD);
    let server_source_channels = [flume::unbounded(), flume::unbounded()];
    let server_sink_channel = flume::unbounded();

    let client_runtimes = [TestRuntime::new(TICK_PERIOD), TestRuntime::new(TICK_PERIOD)];
    let client_source_channels = [flume::unbounded(), flume::unbounded()];
    let client_sink_channels = [flume::unbounded(), flume::unbounded()];

    let mut server = Server::builder(2, Box::new(server_runtime.clone()))
        .sender_with_sockets::<_, 32, 3>(
            &server_to_client_schema,
            server_to_client_mapper_socket,
            enum_map! {
                Mirroring::AudioVideo => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap(),
                Mirroring::Background => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap(),
                Mirroring::Voice => UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap(),
            },
            TestServerToClientSourceFactory {
                channels: [server_source_channels[0].1.clone(), server_source_channels[1].1.clone()],
            },
        )
        .unwrap()
        .receiver_with_socket::<_, 16, 3>(
            &client_to_server_schema,
            client_to_server_mapper_socket,
            client_to_server_socket,
            TestClientToServerSinkFactory {
                channel: server_sink_channel.0.clone(),
            },
        )
        .unwrap()
        .build();
    server.register(server_session_1);
    server.register(server_session_2);

    let _client_1 = Client::builder(client_session_1, Box::new(client_runtimes[0].clone()))
        .sender::<_, 16, 3>(
            &client_to_server_schema,
            TestClientToServerSource {
                channel: client_source_channels[0].1.clone(),
            },
        )
        .unwrap()
        .receiver::<_, 32, 3>(
            &server_to_client_schema,
            TestServerToClientSink {
                channel: client_sink_channels[0].0.clone(),
            },
        )
        .unwrap()
        .build();

    let _client_2 = Client::builder(client_session_2, Box::new(client_runtimes[1].clone()))
        .sender::<_, 16, 3>(
            &client_to_server_schema,
            TestClientToServerSource {
                channel: client_source_channels[1].1.clone(),
            },
        )
        .unwrap()
        .receiver::<_, 32, 3>(
            &server_to_client_schema,
            TestServerToClientSink {
                channel: client_sink_channels[1].0.clone(),
            },
        )
        .unwrap()
        .build();

    // Initial
    {
        assert!(server_sink_channel.1.is_empty());
        assert!(client_sink_channels[0].1.is_empty());
        assert!(client_sink_channels[1].1.is_empty());

        server_runtime.tick();
        client_runtimes[0].tick();
        client_runtimes[1].tick();
        assert!(server_sink_channel.1.is_empty());
        assert!(client_sink_channels[0].1.is_empty());
        assert!(client_sink_channels[1].1.is_empty());
    }

    // Client 1 input
    {
        client_source_channels[0].0.send((1, 10)).unwrap();
        client_runtimes[0].tick();

        server_runtime.tick();
        client_runtimes[0].tick();
        client_runtimes[1].tick();
        assert_eq!(server_sink_channel.1.try_recv().unwrap(), (1, 0, 10));
        assert!(server_sink_channel.1.is_empty());
        assert!(client_sink_channels[0].1.is_empty());
        assert!(client_sink_channels[1].1.is_empty());
    }

    // Client 2 input
    {
        client_source_channels[1].0.send((1, 20)).unwrap();
        client_runtimes[1].tick();

        server_runtime.tick();
        client_runtimes[0].tick();
        client_runtimes[1].tick();
        assert_eq!(server_sink_channel.1.try_recv().unwrap(), (1, 1, 20));
        assert!(server_sink_channel.1.is_empty());
        assert!(client_sink_channels[0].1.is_empty());
        assert!(client_sink_channels[1].1.is_empty());
    }

    // Server input
    {
        server_source_channels[0].0.send((1, [10, 20])).unwrap();
        server_source_channels[1].0.send((1, [10, 20])).unwrap();
        server_runtime.tick();

        server_runtime.tick();
        client_runtimes[0].tick();
        client_runtimes[1].tick();
        assert!(server_sink_channel.1.is_empty());
        assert_eq!(client_sink_channels[0].1.try_recv().unwrap(), (1, [10, 20]));
        assert!(client_sink_channels[0].1.is_empty());
        assert_eq!(client_sink_channels[1].1.try_recv().unwrap(), (1, [10, 20]));
        assert!(client_sink_channels[1].1.is_empty());
    }

    // All inputs (with double tick)
    {
        client_source_channels[0].0.send((2, 30)).unwrap();
        client_source_channels[1].0.send((2, 40)).unwrap();
        server_source_channels[0].0.send((3, [50, 60])).unwrap();
        server_source_channels[1].0.send((3, [50, 60])).unwrap();

        server_runtime.tick();
        client_runtimes[0].tick();
        client_runtimes[1].tick();
        server_runtime.tick();
        client_runtimes[0].tick();
        client_runtimes[1].tick();
        assert_eq!(server_sink_channel.1.try_recv().unwrap(), (2, 0, 30));
        assert_eq!(server_sink_channel.1.try_recv().unwrap(), (2, 1, 40));
        assert!(server_sink_channel.1.is_empty());
        assert_eq!(client_sink_channels[0].1.try_recv().unwrap(), (3, [50, 60]));
        assert!(client_sink_channels[0].1.is_empty());
        assert_eq!(client_sink_channels[1].1.try_recv().unwrap(), (3, [50, 60]));
        assert!(client_sink_channels[1].1.is_empty());
    }
}
