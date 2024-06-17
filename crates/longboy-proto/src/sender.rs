use crate::{Cipher, Source};

pub struct Sender
{
    size: usize,
    window_size: usize,
    cipher: Cipher,
    max_cycle: usize,

    cycle: usize,
    flags: Box<[bool]>, // window_size
    buffer: Box<[u8]>,  // cycle + timestamp + (size * flags.len())

    source: Box<dyn Source>,
}

impl Sender
{
    pub fn new(size: usize, window_size: usize, key: u64, source: Box<dyn Source>) -> Self
    {
        let flags_size = window_size as usize;
        let buffer_size = (std::mem::size_of::<u16>() * 2) + ((size as usize) * flags_size);

        Self {
            size: size,
            window_size: window_size,
            cipher: Cipher::new(key),
            max_cycle: ((u16::MAX as usize) / window_size) * window_size,

            cycle: 0,
            flags: (0..flags_size).map(|_| false).collect(),
            buffer: (0..buffer_size).map(|_| 0).collect(),

            source: source,
        }
    }

    pub fn cycle(&self) -> usize
    {
        self.cycle
    }

    pub fn poll_datagram(&mut self, timestamp: u16) -> Option<&[u8]>
    {
        // Record cycle and timestamp.
        self.buffer.as_chunks_mut().0[0] = (self.cycle as u16).to_le_bytes();
        self.buffer.as_chunks_mut().0[1] = timestamp.to_le_bytes();
        self.cipher.encrypt_header(&mut self.buffer[0..4]);

        // Poll source.
        let index = self.cycle % self.window_size;
        let start = (std::mem::size_of::<u16>() * 2) + (self.size * index);
        let end = start + self.size;
        match self.source.poll(&mut self.buffer[start..end])
        {
            true =>
            {
                self.cipher.encrypt_slot(&mut self.buffer[start..end]);
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
                self.cycle = (self.cycle + 1) % self.max_cycle;
                Some(&self.buffer)
            }
            false => None,
        }
    }
}
