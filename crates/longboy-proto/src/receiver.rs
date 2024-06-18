use crate::{Cipher, Sink};

pub struct Receiver(Box<dyn ReceiverTrait>);

trait ReceiverTrait
{
    fn cycle(&self) -> usize;
    fn handle_datagram(&mut self, timestamp: u16, datagram: &mut [u8]);
}

struct ReceiverImpl<const SIZE: usize, const WINDOW_SIZE: usize>
where
    [(); 0 - (!(SIZE * WINDOW_SIZE < 420) as usize)]:, // assert(SIZE * WINDOW_SIZE < 420)
    [(); (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)]:,
    [(); (((WINDOW_SIZE < 4) as usize) * 8) + (((WINDOW_SIZE >= 4) as usize) * WINDOW_SIZE * 2)]:, // std::cmp::max(8, 2 * WINDOW_SIZE)
{
    cipher: Cipher<SIZE>,
    sink: Box<dyn Sink>,

    cycle: usize,
    flags: [bool; (((WINDOW_SIZE < 4) as usize) * 8) + (((WINDOW_SIZE >= 4) as usize) * WINDOW_SIZE * 2)],
}

impl Receiver
{
    pub fn new(size: usize, window_size: usize, key: u64, sink: Box<dyn Sink>) -> Self
    {
        match (size, window_size)
        {
            (8, 1) => Self(Box::new(ReceiverImpl::<8, 1>::new(key, sink))),
            (8, 3) => Self(Box::new(ReceiverImpl::<8, 3>::new(key, sink))),
            (16, 1) => Self(Box::new(ReceiverImpl::<16, 1>::new(key, sink))),
            (16, 3) => Self(Box::new(ReceiverImpl::<16, 3>::new(key, sink))),
            (32, 1) => Self(Box::new(ReceiverImpl::<32, 1>::new(key, sink))),
            (32, 3) => Self(Box::new(ReceiverImpl::<32, 3>::new(key, sink))),
            (64, 1) => Self(Box::new(ReceiverImpl::<64, 1>::new(key, sink))),
            (64, 3) => Self(Box::new(ReceiverImpl::<64, 3>::new(key, sink))),
            (128, 1) => Self(Box::new(ReceiverImpl::<128, 1>::new(key, sink))),
            (128, 3) => Self(Box::new(ReceiverImpl::<128, 3>::new(key, sink))),
            _ => panic!("Unsupported (size, window_size) tuple"),
        }
    }

    pub fn cycle(&self) -> usize
    {
        self.0.cycle()
    }

    pub fn handle_datagram(&mut self, timestamp: u16, datagram: &mut [u8])
    {
        self.0.handle_datagram(timestamp, datagram)
    }
}

impl<const SIZE: usize, const WINDOW_SIZE: usize> ReceiverImpl<SIZE, WINDOW_SIZE>
where
    [(); 0 - (!(SIZE * WINDOW_SIZE < 420) as usize)]:,
    [(); (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)]:,
    [(); (((WINDOW_SIZE < 4) as usize) * 8) + (((WINDOW_SIZE >= 4) as usize) * WINDOW_SIZE * 2)]:,
{
    const MAX_CYCLE: usize = ((u16::MAX as usize) / WINDOW_SIZE) * WINDOW_SIZE;

    pub fn new(key: u64, sink: Box<dyn Sink>) -> Self
    {
        Self {
            cipher: Cipher::new(key),
            sink: sink,

            cycle: 0,
            flags: [false; (((WINDOW_SIZE < 4) as usize) * 8) + (((WINDOW_SIZE >= 4) as usize) * WINDOW_SIZE * 2)],
        }
    }
}

impl<const SIZE: usize, const WINDOW_SIZE: usize> ReceiverTrait for ReceiverImpl<SIZE, WINDOW_SIZE>
where
    [(); 0 - (!(SIZE * WINDOW_SIZE < 420) as usize)]:,
    [(); (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)]:,
    [(); (((WINDOW_SIZE < 4) as usize) * 8) + (((WINDOW_SIZE >= 4) as usize) * WINDOW_SIZE * 2)]:,
{
    fn cycle(&self) -> usize
    {
        self.cycle
    }

    fn handle_datagram(&mut self, timestamp: u16, datagram: &mut [u8])
    {
        let datagram: &mut [u8; (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)] = match datagram.try_into()
        {
            Ok(datagram) => datagram,
            Err(_) => return,
        };

        // Grab cycle and timestamp.
        self.cipher
            .decrypt_header(<&mut [u8; 4]>::try_from(&mut datagram[0..4]).unwrap());
        let datagram_cycle = u16::from_le_bytes(*<&[u8; 2]>::try_from(&datagram[0..2]).unwrap()) as usize;
        let datagram_timestamp = u16::from_le_bytes(*<&[u8; 2]>::try_from(&datagram[2..4]).unwrap());

        // Calculate diff for cycle and timestamp.
        let cycle_diff = ((datagram_cycle + Self::MAX_CYCLE) - self.cycle) % Self::MAX_CYCLE;
        let timestamp_diff = ((datagram_timestamp + u16::MAX) - timestamp) % u16::MAX;

        // Check for bad datagrams or late datagrams that are already processed.  Because
        // we ensure only a positive diff, this is done by checking for any values greater
        // that a certain threshold.
        if cycle_diff > 256 || timestamp_diff > 2048
        {
            // Bad datagram or already received.
            return;
        }

        // Check for late or missing packets from between local cycle and the datagram
        // cycle just received.
        if cycle_diff > std::cmp::min(8, WINDOW_SIZE + 1)
        {
            // soft warning
        }
        if cycle_diff > self.flags.len()
        {
            // hard warning

            for _ in 0..(cycle_diff - self.flags.len())
            {
                let index = self.cycle % self.flags.len();
                self.flags[index] = false;
                self.cycle = (self.cycle + 1) % Self::MAX_CYCLE;
            }
        }

        // Sink input.
        for i in 0..WINDOW_SIZE
        {
            let cycle_i = ((datagram_cycle + Self::MAX_CYCLE) - i) % Self::MAX_CYCLE;

            // If we're before local cycle, early out.  This is effectively checking for distance
            // being out of the buffer's size, which is only possible if before because we've
            // already adanced the local cycle to catch up, if applicable.
            if ((cycle_i + Self::MAX_CYCLE) - self.cycle) % Self::MAX_CYCLE > self.flags.len()
            {
                break;
            }

            let source_index = cycle_i % WINDOW_SIZE;
            let destination_index = cycle_i % self.flags.len();

            if !self.flags[destination_index]
            {
                let start = (std::mem::size_of::<u16>() * 2) + (SIZE * source_index);
                let end = start + SIZE;
                self.cipher
                    .decrypt_slot(<&mut [u8; SIZE]>::try_from(&mut datagram[start..end]).unwrap());
                self.sink.handle(&datagram[start..end]);
                self.flags[destination_index] = true;
            }
        }

        // Advance cycles.
        loop
        {
            let index = self.cycle % self.flags.len();
            if !self.flags[index]
            {
                break;
            }
            self.flags[index] = false;
            self.cycle = (self.cycle + 1) % Self::MAX_CYCLE;
        }
    }
}
