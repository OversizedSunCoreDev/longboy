use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use longboy_proto::{Receiver, Sender, Sink, Source};

struct TestSource
{
    counter: Arc<AtomicU64>,
    accumulator: u64,
    period: u64,
}

struct TestSink
{
    counter: Arc<AtomicU64>,
}

impl Source for TestSource
{
    fn poll(&mut self, buffer: &mut [u8]) -> bool
    {
        self.accumulator += 1;
        match self.accumulator % self.period == 0
        {
            true =>
            {
                let counter = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
                *buffer.first_chunk_mut().unwrap() = u64::to_le_bytes(counter);
                true
            }
            false => false,
        }
    }
}

impl Sink for TestSink
{
    fn handle(&mut self, input: &[u8])
    {
        let counter = u64::from_le_bytes(*input.first_chunk().unwrap());
        self.counter.fetch_max(counter, Ordering::Relaxed);
    }
}

#[test]
fn golden()
{
    let size = 8;
    let window_size = 3;
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender = Sender::new(
        size,
        window_size,
        key,
        Box::new(TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        }),
    );
    let mut receiver = Receiver::new(
        size,
        window_size,
        key,
        Box::new(TestSink {
            counter: sink_counter.clone(),
        }),
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    let mut datagram_1: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 1);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 1);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 0);

    receiver.handle_datagram(timestamp, &mut datagram_1);
    assert_eq!(sender.cycle(), 1);
    assert_eq!(receiver.cycle(), 1);
    assert_eq!(source_counter.load(Ordering::Relaxed), 1);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 1);

    let mut datagram_2: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 2);
    assert_eq!(receiver.cycle(), 1);
    assert_eq!(source_counter.load(Ordering::Relaxed), 2);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 1);

    receiver.handle_datagram(timestamp, &mut datagram_2);
    assert_eq!(sender.cycle(), 2);
    assert_eq!(receiver.cycle(), 2);
    assert_eq!(source_counter.load(Ordering::Relaxed), 2);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 2);

    let mut datagram_3: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 2);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 2);

    receiver.handle_datagram(timestamp, &mut datagram_3);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 3);

    let mut datagram_4: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 4);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 4);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 3);

    receiver.handle_datagram(timestamp, &mut datagram_4);
    assert_eq!(sender.cycle(), 4);
    assert_eq!(receiver.cycle(), 4);
    assert_eq!(source_counter.load(Ordering::Relaxed), 4);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 4);

    let mut datagram_5: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 5);
    assert_eq!(receiver.cycle(), 4);
    assert_eq!(source_counter.load(Ordering::Relaxed), 5);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 4);

    receiver.handle_datagram(timestamp, &mut datagram_5);
    assert_eq!(sender.cycle(), 5);
    assert_eq!(receiver.cycle(), 5);
    assert_eq!(source_counter.load(Ordering::Relaxed), 5);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 5);

    let mut datagram_6: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 6);
    assert_eq!(receiver.cycle(), 5);
    assert_eq!(source_counter.load(Ordering::Relaxed), 6);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 5);

    receiver.handle_datagram(timestamp, &mut datagram_6);
    assert_eq!(sender.cycle(), 6);
    assert_eq!(receiver.cycle(), 6);
    assert_eq!(source_counter.load(Ordering::Relaxed), 6);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 6);
}

#[test]
fn mirroring()
{
    let size = 8;
    let window_size = 3;
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender = Sender::new(
        size,
        window_size,
        key,
        Box::new(TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        }),
    );
    let mut receiver = Receiver::new(
        size,
        window_size,
        key,
        Box::new(TestSink {
            counter: sink_counter.clone(),
        }),
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    let mut datagram_1: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_2: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_3: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 0);

    receiver.handle_datagram(timestamp, &mut datagram_1.clone());
    receiver.handle_datagram(timestamp, &mut datagram_1.clone());
    receiver.handle_datagram(timestamp, &mut datagram_1);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 1);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 1);

    receiver.handle_datagram(timestamp, &mut datagram_2.clone());
    receiver.handle_datagram(timestamp, &mut datagram_2.clone());
    receiver.handle_datagram(timestamp, &mut datagram_2);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 2);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 2);

    receiver.handle_datagram(timestamp, &mut datagram_3.clone());
    receiver.handle_datagram(timestamp, &mut datagram_3.clone());
    receiver.handle_datagram(timestamp, &mut datagram_3);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 3);
}

#[test]
fn out_of_order()
{
    let size = 8;
    let window_size = 3;
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender = Sender::new(
        size,
        window_size,
        key,
        Box::new(TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        }),
    );
    let mut receiver = Receiver::new(
        size,
        window_size,
        key,
        Box::new(TestSink {
            counter: sink_counter.clone(),
        }),
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    // Inside of window.
    let mut datagram_1: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_2: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_3: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 0);

    receiver.handle_datagram(timestamp, &mut datagram_3);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 3);

    receiver.handle_datagram(timestamp, &mut datagram_2);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 3);

    receiver.handle_datagram(timestamp, &mut datagram_1);
    assert_eq!(sender.cycle(), 3);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 3);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 3);

    // Outside of window.
    let mut datagram_4: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_5: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_6: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_7: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_8: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_9: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_10: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    let mut datagram_11: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());

    receiver.handle_datagram(timestamp, &mut datagram_11);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_10);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_9);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_8);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_7);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 3);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_6);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 11);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_5);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 11);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);

    receiver.handle_datagram(timestamp, &mut datagram_4);
    assert_eq!(sender.cycle(), 11);
    assert_eq!(receiver.cycle(), 11);
    assert_eq!(source_counter.load(Ordering::Relaxed), 11);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 11);
}

#[test]
fn lost_in_transmission()
{
    let size = 8;
    let window_size = 3;
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender = Sender::new(
        size,
        window_size,
        key,
        Box::new(TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        }),
    );
    let mut receiver = Receiver::new(
        size,
        window_size,
        key,
        Box::new(TestSink {
            counter: sink_counter.clone(),
        }),
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    for _ in 0..128
    {
        sender.poll_datagram(timestamp).unwrap();
    }
    assert_eq!(sender.cycle(), 128);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 128);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 0);

    let mut datagram: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 129);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 129);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 0);

    receiver.handle_datagram(timestamp, &mut datagram);
    assert_eq!(sender.cycle(), 129);
    assert_eq!(receiver.cycle(), 121);
    assert_eq!(source_counter.load(Ordering::Relaxed), 129);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 129);
}

#[test]
fn cycle_wrapping()
{
    let size = 8;
    let window_size = 3;
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender = Sender::new(
        size,
        window_size,
        key,
        Box::new(TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        }),
    );
    let mut receiver = Receiver::new(
        size,
        window_size,
        key,
        Box::new(TestSink {
            counter: sink_counter.clone(),
        }),
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    for _ in 0..(65535 - 1)
    {
        let mut datagram: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
        receiver.handle_datagram(timestamp, &mut datagram);
    }
    assert_eq!(sender.cycle(), 65534);
    assert_eq!(receiver.cycle(), 65534);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65534);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65534);

    let mut datagram: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 65534);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65535);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65534);

    receiver.handle_datagram(timestamp, &mut datagram);
    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65535);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65535);

    let mut datagram: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 1);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65536);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65535);

    receiver.handle_datagram(timestamp, &mut datagram);
    assert_eq!(sender.cycle(), 1);
    assert_eq!(receiver.cycle(), 1);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65536);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65536);
}

#[test]
fn sparse()
{
    let size = 8;
    let window_size = 3;
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));

    let period = 300;

    let timestamp = 0;

    let mut sender = Sender::new(
        size,
        window_size,
        key,
        Box::new(TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: period,
        }),
    );
    let mut receiver = Receiver::new(
        size,
        window_size,
        key,
        Box::new(TestSink {
            counter: sink_counter.clone(),
        }),
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    let mut accumulator = 0;

    while accumulator < (period - 1)
    {
        accumulator += 1;

        let datagram = sender.poll_datagram(timestamp);
        assert!(datagram.is_none());
        assert_eq!(sender.cycle(), 0);
        assert_eq!(receiver.cycle(), 0);
        assert_eq!(source_counter.load(Ordering::Relaxed), 0);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 0);
    }

    for i in 0..window_size
    {
        accumulator += 1;

        let mut datagram: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
        receiver.handle_datagram(timestamp, &mut datagram);
        assert_eq!(sender.cycle(), 1 + i);
        assert_eq!(receiver.cycle(), 1 + i);
        assert_eq!(source_counter.load(Ordering::Relaxed), 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 1);
    }

    while accumulator < ((period * 2) - 1)
    {
        accumulator += 1;

        let datagram = sender.poll_datagram(timestamp);
        assert!(datagram.is_none());
        assert_eq!(sender.cycle(), window_size);
        assert_eq!(receiver.cycle(), window_size);
        assert_eq!(source_counter.load(Ordering::Relaxed), 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 1);
    }

    for i in 0..window_size
    {
        accumulator += 1;

        let mut datagram: Box<[u8]> = Box::from(sender.poll_datagram(timestamp).unwrap());
        receiver.handle_datagram(timestamp, &mut datagram);
        assert_eq!(source_counter.load(Ordering::Relaxed), 2);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 2);
        assert_eq!(sender.cycle(), window_size + 1 + i);
        assert_eq!(receiver.cycle(), window_size + 1 + i);
    }
}
