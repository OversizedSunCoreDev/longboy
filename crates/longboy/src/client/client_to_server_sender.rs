use std::net::{SocketAddr, UdpSocket};

use anyhow::Result;
use enum_map::{Enum, EnumMap};

use crate::{Constants, Mirroring, RuntimeTask, Sender, Source, UdpSocketExt};

pub(crate) struct ClientToServerSender<SourceType, const SIZE: usize, const WINDOW_SIZE: usize>
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    name: String,

    mapper_socket_addr: SocketAddr,
    heartbeat_period: u16,
    socket_addr: SocketAddr,

    sockets: EnumMap<Mirroring, UdpSocket>,

    session_id: u64,
    next_heartbeat: u16,
    sender: Sender<SourceType, SIZE, WINDOW_SIZE>,
}

impl<SourceType, const SIZE: usize, const WINDOW_SIZE: usize> ClientToServerSender<SourceType, SIZE, WINDOW_SIZE>
where
    SourceType: Source<SIZE>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        name: String,
        mapper_socket_addr: SocketAddr,
        heartbeat_period: u16,
        socket_addr: SocketAddr,
        session_id: u64,
        cipher_key: u64,
        sockets: EnumMap<Mirroring, UdpSocket>,
        source: SourceType,
    ) -> Result<Self>
    {
        sockets[Mirroring::AudioVideo].set_nonblocking(true)?;
        sockets[Mirroring::AudioVideo].set_qos_audio_video()?;

        sockets[Mirroring::Background].set_nonblocking(true)?;
        sockets[Mirroring::Background].set_qos_background()?;

        sockets[Mirroring::Voice].set_nonblocking(true)?;
        sockets[Mirroring::Voice].set_qos_voice()?;

        Ok(Self {
            name,

            mapper_socket_addr,
            heartbeat_period,
            socket_addr,

            sockets,

            session_id,
            next_heartbeat: 0,
            sender: Sender::new(cipher_key, source),
        })
    }
}

impl<SourceType, const SIZE: usize, const WINDOW_SIZE: usize> RuntimeTask
    for ClientToServerSender<SourceType, SIZE, WINDOW_SIZE>
where
    SourceType: Source<SIZE>,
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
{
    fn name(&self) -> &str
    {
        &self.name
    }

    fn poll(&mut self, timestamp: u16)
    {
        // Heartbeat to Server.
        if timestamp >= self.next_heartbeat
        {
            let mut buffer = [0; std::mem::size_of::<u64>() + std::mem::size_of::<u8>()];
            *<&mut [u8; 8]>::try_from(&mut buffer[0..8]).unwrap() = self.session_id.to_le_bytes();

            for (mirroring, socket) in self.sockets.iter()
            {
                buffer[8] = Mirroring::into_usize(mirroring) as u8;
                // debug sent buffer
                print!("Sending heartbeat buffer: {:?}", buffer);
                socket
                    .send_to(&buffer, self.mapper_socket_addr)
                    .expect("send_to failure");
            }

            self.next_heartbeat = timestamp + self.heartbeat_period;
        }

        // Poll Session
        if let Some(datagram) = self.sender.poll_datagram(timestamp)
        {
            print!("Sending datagram: {:?}", datagram);
            for socket in self.sockets.values()
            {
                // debug sent datagram
                socket.send_to(datagram, self.socket_addr).expect("send_to failure");
            }
        }
    }
}
