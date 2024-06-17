use crate::{Cipher, Sink};

pub struct Receiver
{
    size: usize,
    window_size: usize,
    cipher: Cipher,
    max_cycle: usize,

    cycle: usize,
    flags: Box<[bool]>, // max(8, window_size * 2)

    sink: Box<dyn Sink>,
}

impl Receiver
{
    pub fn new(size: usize, window_size: usize, key: u64, sink: Box<dyn Sink>) -> Self
    {
        let flags_size = std::cmp::max(8, 2 * window_size);

        Self {
            size: size,
            window_size: window_size,
            cipher: Cipher::new(key),
            max_cycle: ((u16::MAX as usize) / window_size) * window_size,

            cycle: 0,
            flags: (0..flags_size).map(|_| false).collect(),

            sink: sink,
        }
    }

    pub fn cycle(&self) -> usize
    {
        self.cycle
    }

    pub fn handle_datagram(&mut self, timestamp: u16, datagram: &mut [u8])
    {
        // Grab cycle and timestamp.
        self.cipher.decrypt_header(&mut datagram[0..4]);
        let datagram_cycle = u16::from_le_bytes(datagram.as_chunks().0[0]) as usize;
        let datagram_timestamp = u16::from_le_bytes(datagram.as_chunks().0[1]);

        // Calculate diff for cycle and timestamp.
        let cycle_diff = ((datagram_cycle + self.max_cycle) - self.cycle) % self.max_cycle;
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
        if cycle_diff > std::cmp::min(8, self.window_size + 1)
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
                self.cycle = (self.cycle + 1) % self.max_cycle;
            }
        }

        // Sink input.
        for i in 0..self.window_size
        {
            let cycle_i = ((datagram_cycle + self.max_cycle) - i) % self.max_cycle;

            // If we're before local cycle, early out.  This is effectively checking for distance
            // being out of the buffer's size, which is only possible if before because we've
            // already adanced the local cycle to catch up, if applicable.
            if ((cycle_i + self.max_cycle) - self.cycle) % self.max_cycle > self.flags.len()
            {
                break;
            }

            let source_index = cycle_i % self.window_size;
            let destination_index = cycle_i % self.flags.len();

            if !self.flags[destination_index]
            {
                let start = (std::mem::size_of::<u16>() * 2) + (self.size * source_index);
                let end = start + self.size;
                self.cipher.decrypt_slot(&mut datagram[start..end]);
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
            self.cycle = (self.cycle + 1) % self.max_cycle;
        }
    }
}
