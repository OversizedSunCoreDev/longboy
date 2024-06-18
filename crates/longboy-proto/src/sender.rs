use crate::{Cipher, Source};

pub struct Sender(Box<dyn SenderTrait>);

trait SenderTrait
{
    fn cycle(&self) -> usize;
    fn poll_datagram(&mut self, timestamp: u16) -> Option<&[u8]>;
}

struct SenderImpl<const SIZE: usize, const WINDOW_SIZE: usize>
where
    [(); 0 - (!(SIZE * WINDOW_SIZE < 420) as usize)]:, // assert(SIZE * WINDOW_SIZE < 420)
    [(); (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)]:,
{
    cipher: Cipher<SIZE>,
    source: Box<dyn Source>,

    cycle: usize,
    flags: [bool; WINDOW_SIZE],
    buffer: [u8; (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)],
}

impl Sender
{
    pub fn new(size: usize, window_size: usize, key: u64, source: Box<dyn Source>) -> Self
    {
        match (size, window_size)
        {
            (8, 1) => Self(Box::new(SenderImpl::<8, 1>::new(key, source))),
            (8, 3) => Self(Box::new(SenderImpl::<8, 3>::new(key, source))),
            (16, 1) => Self(Box::new(SenderImpl::<16, 1>::new(key, source))),
            (16, 3) => Self(Box::new(SenderImpl::<16, 3>::new(key, source))),
            (32, 1) => Self(Box::new(SenderImpl::<32, 1>::new(key, source))),
            (32, 3) => Self(Box::new(SenderImpl::<32, 3>::new(key, source))),
            (64, 1) => Self(Box::new(SenderImpl::<64, 1>::new(key, source))),
            (64, 3) => Self(Box::new(SenderImpl::<64, 3>::new(key, source))),
            (128, 1) => Self(Box::new(SenderImpl::<128, 1>::new(key, source))),
            (128, 3) => Self(Box::new(SenderImpl::<128, 3>::new(key, source))),
            _ => panic!("Unsupported (size, window_size) tuple"),
        }
    }

    pub fn cycle(&self) -> usize
    {
        self.0.cycle()
    }

    pub fn poll_datagram(&mut self, timestamp: u16) -> Option<&[u8]>
    {
        self.0.poll_datagram(timestamp)
    }
}

impl<const SIZE: usize, const WINDOW_SIZE: usize> SenderImpl<SIZE, WINDOW_SIZE>
where
    [(); 0 - (!(SIZE * WINDOW_SIZE < 420) as usize)]:,
    [(); (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)]:,
{
    const MAX_CYCLE: usize = ((u16::MAX as usize) / WINDOW_SIZE) * WINDOW_SIZE;

    fn new(key: u64, source: Box<dyn Source>) -> Self
    {
        Self {
            cipher: Cipher::new(key),
            source: source,

            cycle: 0,
            flags: [false; WINDOW_SIZE],
            buffer: [0; (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)],
        }
    }
}

impl<const SIZE: usize, const WINDOW_SIZE: usize> SenderTrait for SenderImpl<SIZE, WINDOW_SIZE>
where
    [(); 0 - (!(SIZE * WINDOW_SIZE < 420) as usize)]:,
    [(); (std::mem::size_of::<u16>() * 2) + (SIZE * WINDOW_SIZE)]:,
{
    fn cycle(&self) -> usize
    {
        self.cycle
    }

    fn poll_datagram(&mut self, timestamp: u16) -> Option<&[u8]>
    {
        // Record cycle and timestamp.
        *<&mut [u8; 2]>::try_from(&mut self.buffer[0..2]).unwrap() = (self.cycle as u16).to_le_bytes();
        *<&mut [u8; 2]>::try_from(&mut self.buffer[2..4]).unwrap() = timestamp.to_le_bytes();
        self.cipher
            .encrypt_header(<&mut [u8; 4]>::try_from(&mut self.buffer[0..4]).unwrap());

        // Poll source.
        let index = self.cycle % WINDOW_SIZE;
        let start = (std::mem::size_of::<u16>() * 2) + (SIZE * index);
        let end = start + SIZE;
        match self.source.poll(&mut self.buffer[start..end])
        {
            true =>
            {
                self.cipher
                    .encrypt_slot(<&mut [u8; SIZE]>::try_from(&mut self.buffer[start..end]).unwrap());
                self.flags[index] = true;
            }
            false =>
            {
                self.buffer[start..end].fill(0);
                self.flags[index] = false;
            }
        }
        if !self.flags[index]
        {}

        // Check for transmit and potentially advance cycle.
        match self.flags.iter().any(|flag| *flag)
        {
            true =>
            {
                self.cycle = (self.cycle + 1) % Self::MAX_CYCLE;
                Some(&self.buffer)
            }
            false => None,
        }
    }
}
