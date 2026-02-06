use std::net::{SocketAddr, UdpSocket};

use anyhow::Result;

use crate::{Constants, Receiver, RuntimeTask, Sink};

pub(crate) struct ServerToClientReceiver<SinkType, const SIZE: usize, const WINDOW_SIZE: usize>
where
    SinkType: Sink<SIZE>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    name: String,

    mapper_socket_addr: SocketAddr,
    heartbeat_period: u16,

    socket: UdpSocket,

    session_id: u64,
    next_heartbeat: u16,
    receiver: Receiver<SinkType, SIZE, WINDOW_SIZE>,
}

impl<SinkType, const SIZE: usize, const WINDOW_SIZE: usize> ServerToClientReceiver<SinkType, SIZE, WINDOW_SIZE>
where
    SinkType: Sink<SIZE>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    pub(crate) fn new(
        name: String,
        mapper_socket_addr: SocketAddr,
        heartbeat_period: u16,
        session_id: u64,
        cipher_key: u64,
        socket: UdpSocket,
        sink: SinkType,
    ) -> Result<Self>
    {
        socket.set_nonblocking(true)?;

        Ok(Self {
            name,

            mapper_socket_addr,
            heartbeat_period,

            socket,

            session_id,
            next_heartbeat: 0,
            receiver: Receiver::new(cipher_key, sink),
        })
    }
}

impl<SinkType, const SIZE: usize, const WINDOW_SIZE: usize> RuntimeTask
    for ServerToClientReceiver<SinkType, SIZE, WINDOW_SIZE>
where
    SinkType: Sink<SIZE>,
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

        // Heartbeat to Server.
        if timestamp >= self.next_heartbeat
        {
            let mut buffer = [0; std::mem::size_of::<u64>()];
            *<&mut [u8; 8]>::try_from(&mut buffer[0..8]).unwrap() = self.session_id.to_le_bytes();

            self.socket
                .send_to(&buffer, self.mapper_socket_addr)
                .expect("send_to failure");

            self.next_heartbeat = timestamp + self.heartbeat_period;
        }

        // Process datagrams.
        let mut buffer = [0; 512];
        while let Ok((len, _)) = self.socket.recv_from(&mut buffer)
        {
            if len != DATAGRAM_SIZE
            {
                continue;
            }

            let datagram = (&mut buffer[0..DATAGRAM_SIZE]).try_into().unwrap();

            self.receiver.handle_datagram(timestamp, datagram);
        }
    }
}
