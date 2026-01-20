use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use longboy::{Constants, Receiver, Sender, Sink, Source};

struct TestSource
{
    counter: Arc<AtomicU64>,
    accumulator: u64,
    period: u64,
}

struct TestSink
{
    counter: Arc<AtomicU64>,
    handled: Arc<AtomicU64>,
}

impl<const SIZE: usize> Source<SIZE> for TestSource
{
    fn poll(&mut self, buffer: &mut [u8; SIZE]) -> bool
    {
        self.accumulator += 1;
        match self.accumulator.is_multiple_of(self.period)
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

impl<const SIZE: usize> Sink<SIZE> for TestSink
{
    fn handle(&mut self, buffer: &[u8; SIZE])
    {
        let counter = u64::from_le_bytes(*buffer.first_chunk().unwrap());
        self.counter.fetch_max(counter, Ordering::Relaxed);
        self.handled.fetch_add(1, Ordering::Relaxed);
    }
}

macro_rules! test {
    ($func:ident) => {
        mod $func
        {
            #[test]
            fn size_8_window_size_1()
            {
                super::$func::<8, 1>()
            }

            #[test]
            fn size_16_window_size_1()
            {
                super::$func::<16, 1>()
            }

            #[test]
            fn size_32_window_size_1()
            {
                super::$func::<32, 1>()
            }

            #[test]
            fn size_64_window_size_1()
            {
                super::$func::<64, 1>()
            }

            #[test]
            fn size_128_window_size_1()
            {
                super::$func::<128, 1>()
            }

            #[test]
            fn size_8_window_size_3()
            {
                super::$func::<8, 3>()
            }

            #[test]
            fn size_16_window_size_3()
            {
                super::$func::<16, 3>()
            }

            #[test]
            fn size_32_window_size_3()
            {
                super::$func::<32, 3>()
            }

            #[test]
            fn size_64_window_size_3()
            {
                super::$func::<64, 3>()
            }

            #[test]
            fn size_128_window_size_3()
            {
                super::$func::<128, 3>()
            }
        }
    };
}

test!(golden);
test!(mirroring);
test!(out_of_order);
test!(lost_in_transmission);
test!(cycle_wrapping);
test!(sparse);

fn golden<const SIZE: usize, const WINDOW_SIZE: usize>()
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));
    let handled_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
        key,
        TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        },
    );
    let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
        key,
        TestSink {
            counter: sink_counter.clone(),
            handled: handled_counter.clone(),
        },
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    for i in 0..1024
    {
        let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
        assert_eq!(sender.cycle(), i + 1);
        assert_eq!(receiver.cycle(), i);
        assert_eq!(source_counter.load(Ordering::Relaxed), (i as u64) + 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), i as u64);
        assert_eq!(handled_counter.load(Ordering::Relaxed), i as u64);

        receiver.handle_datagram(timestamp, &mut datagram);
        assert_eq!(sender.cycle(), i + 1);
        assert_eq!(receiver.cycle(), i + 1);
        assert_eq!(source_counter.load(Ordering::Relaxed), (i as u64) + 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), (i as u64) + 1);
        assert_eq!(handled_counter.load(Ordering::Relaxed), (i as u64) + 1);
    }
}

fn mirroring<const SIZE: usize, const WINDOW_SIZE: usize>()
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));
    let handled_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
        key,
        TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        },
    );
    let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
        key,
        TestSink {
            counter: sink_counter.clone(),
            handled: handled_counter.clone(),
        },
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    for i in 0..1024
    {
        let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
        assert_eq!(sender.cycle(), i + 1);
        assert_eq!(receiver.cycle(), i);
        assert_eq!(source_counter.load(Ordering::Relaxed), (i as u64) + 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), i as u64);
        assert_eq!(handled_counter.load(Ordering::Relaxed), i as u64);

        receiver.handle_datagram(timestamp, &mut datagram.clone());
        receiver.handle_datagram(timestamp, &mut datagram.clone());
        receiver.handle_datagram(timestamp, &mut datagram);
        assert_eq!(sender.cycle(), i + 1);
        assert_eq!(receiver.cycle(), i + 1);
        assert_eq!(source_counter.load(Ordering::Relaxed), (i as u64) + 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), (i as u64) + 1);
        assert_eq!(handled_counter.load(Ordering::Relaxed), (i as u64) + 1);
    }
}

fn out_of_order<const SIZE: usize, const WINDOW_SIZE: usize>()
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    // Alias constants so they're less painful to read.
    #[allow(non_snake_case)]
    let MAX_BUFFERED: usize = Constants::<SIZE, WINDOW_SIZE>::MAX_BUFFERED;

    // Inside of window.
    {
        let key = 0xDEADBEEFDEADBEEF;

        let source_counter = Arc::new(AtomicU64::new(0));
        let sink_counter = Arc::new(AtomicU64::new(0));
        let handled_counter = Arc::new(AtomicU64::new(0));

        let timestamp = 0;

        let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
            key,
            TestSource {
                counter: source_counter.clone(),
                accumulator: 0,
                period: 1,
            },
        );
        let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
            key,
            TestSink {
                counter: sink_counter.clone(),
                handled: handled_counter.clone(),
            },
        );

        assert_eq!(sender.cycle(), 0);
        assert_eq!(receiver.cycle(), 0);

        let mut datagrams = [0; WINDOW_SIZE].map(|_| Box::new(*sender.poll_datagram(timestamp).unwrap()));
        assert_eq!(sender.cycle(), WINDOW_SIZE);
        assert_eq!(receiver.cycle(), 0);
        assert_eq!(source_counter.load(Ordering::Relaxed), WINDOW_SIZE as u64);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 0);
        assert_eq!(handled_counter.load(Ordering::Relaxed), 0);

        for datagram in datagrams.iter_mut().rev()
        {
            receiver.handle_datagram(timestamp, datagram);
            assert_eq!(sender.cycle(), WINDOW_SIZE);
            assert_eq!(receiver.cycle(), WINDOW_SIZE);
            assert_eq!(source_counter.load(Ordering::Relaxed), WINDOW_SIZE as u64);
            assert_eq!(sink_counter.load(Ordering::Relaxed), WINDOW_SIZE as u64);
            assert_eq!(handled_counter.load(Ordering::Relaxed), WINDOW_SIZE as u64);
        }
    }

    // Outside of window.
    {
        let key = 0xDEADBEEFDEADBEEF;

        let source_counter = Arc::new(AtomicU64::new(0));
        let sink_counter = Arc::new(AtomicU64::new(0));
        let handled_counter = Arc::new(AtomicU64::new(0));

        let timestamp = 0;

        let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
            key,
            TestSource {
                counter: source_counter.clone(),
                accumulator: 0,
                period: 1,
            },
        );
        let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
            key,
            TestSink {
                counter: sink_counter.clone(),
                handled: handled_counter.clone(),
            },
        );

        assert_eq!(sender.cycle(), 0);
        assert_eq!(receiver.cycle(), 0);

        let mut datagrams = [0; <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]
            .map(|_| Box::new(*sender.poll_datagram(timestamp).unwrap()));
        assert_eq!(sender.cycle(), MAX_BUFFERED);
        assert_eq!(receiver.cycle(), 0);
        assert_eq!(source_counter.load(Ordering::Relaxed), (MAX_BUFFERED as u64));
        assert_eq!(sink_counter.load(Ordering::Relaxed), 0);
        assert_eq!(handled_counter.load(Ordering::Relaxed), 0);

        let mut handled = WINDOW_SIZE;
        for datagram in datagrams.iter_mut().rev().take(MAX_BUFFERED - WINDOW_SIZE)
        {
            receiver.handle_datagram(timestamp, datagram);
            assert_eq!(sender.cycle(), MAX_BUFFERED);
            assert_eq!(receiver.cycle(), 0);
            assert_eq!(source_counter.load(Ordering::Relaxed), MAX_BUFFERED as u64);
            assert_eq!(sink_counter.load(Ordering::Relaxed), MAX_BUFFERED as u64);
            assert_eq!(handled_counter.load(Ordering::Relaxed), handled as u64);
            handled = std::cmp::min(handled + 1, MAX_BUFFERED);
        }
        for datagram in datagrams.iter_mut().rev().skip(MAX_BUFFERED - WINDOW_SIZE)
        {
            receiver.handle_datagram(timestamp, datagram);
            assert_eq!(sender.cycle(), MAX_BUFFERED);
            assert_eq!(receiver.cycle(), MAX_BUFFERED);
            assert_eq!(source_counter.load(Ordering::Relaxed), MAX_BUFFERED as u64);
            assert_eq!(sink_counter.load(Ordering::Relaxed), MAX_BUFFERED as u64);
            assert_eq!(handled_counter.load(Ordering::Relaxed), handled as u64);
            handled = std::cmp::min(handled + 1, MAX_BUFFERED);
        }
    }
}

fn lost_in_transmission<const SIZE: usize, const WINDOW_SIZE: usize>()
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));
    let handled_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
        key,
        TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        },
    );
    let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
        key,
        TestSink {
            counter: sink_counter.clone(),
            handled: handled_counter.clone(),
        },
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

    let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 129);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 129);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 0);
    assert_eq!(handled_counter.load(Ordering::Relaxed), 0);

    receiver.handle_datagram(timestamp, &mut datagram);
    assert_eq!(sender.cycle(), 129);
    assert_eq!(receiver.cycle(), 121);
    assert_eq!(source_counter.load(Ordering::Relaxed), 129);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 129);
    assert_eq!(handled_counter.load(Ordering::Relaxed), WINDOW_SIZE as u64);
}

fn cycle_wrapping<const SIZE: usize, const WINDOW_SIZE: usize>()
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));
    let handled_counter = Arc::new(AtomicU64::new(0));

    let timestamp = 0;

    let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
        key,
        TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period: 1,
        },
    );
    let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
        key,
        TestSink {
            counter: sink_counter.clone(),
            handled: handled_counter.clone(),
        },
    );

    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);

    for _ in 0..(65535 - 1)
    {
        let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
        receiver.handle_datagram(timestamp, &mut datagram);
    }
    assert_eq!(sender.cycle(), 65534);
    assert_eq!(receiver.cycle(), 65534);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65534);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65534);
    assert_eq!(handled_counter.load(Ordering::Relaxed), 65534);

    let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 65534);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65535);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65534);
    assert_eq!(handled_counter.load(Ordering::Relaxed), 65534);

    receiver.handle_datagram(timestamp, &mut datagram);
    assert_eq!(sender.cycle(), 0);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65535);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65535);
    assert_eq!(handled_counter.load(Ordering::Relaxed), 65535);

    let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
    assert_eq!(sender.cycle(), 1);
    assert_eq!(receiver.cycle(), 0);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65536);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65535);
    assert_eq!(handled_counter.load(Ordering::Relaxed), 65535);

    receiver.handle_datagram(timestamp, &mut datagram);
    assert_eq!(sender.cycle(), 1);
    assert_eq!(receiver.cycle(), 1);
    assert_eq!(source_counter.load(Ordering::Relaxed), 65536);
    assert_eq!(sink_counter.load(Ordering::Relaxed), 65536);
    assert_eq!(handled_counter.load(Ordering::Relaxed), 65536);
}

fn sparse<const SIZE: usize, const WINDOW_SIZE: usize>()
where
    [(); <Constants<SIZE, WINDOW_SIZE>>::DATAGRAM_SIZE]:,
    [(); <Constants<SIZE, WINDOW_SIZE>>::MAX_BUFFERED]:,
{
    let key = 0xDEADBEEFDEADBEEF;

    let source_counter = Arc::new(AtomicU64::new(0));
    let sink_counter = Arc::new(AtomicU64::new(0));
    let handled_counter = Arc::new(AtomicU64::new(0));

    let period = 300;

    let timestamp = 0;

    let mut sender: Sender<TestSource, SIZE, WINDOW_SIZE> = Sender::new(
        key,
        TestSource {
            counter: source_counter.clone(),
            accumulator: 0,
            period,
        },
    );
    let mut receiver: Receiver<TestSink, SIZE, WINDOW_SIZE> = Receiver::new(
        key,
        TestSink {
            counter: sink_counter.clone(),
            handled: handled_counter.clone(),
        },
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
        assert_eq!(handled_counter.load(Ordering::Relaxed), 0);
    }

    for i in 0..WINDOW_SIZE
    {
        accumulator += 1;

        let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
        receiver.handle_datagram(timestamp, &mut datagram);
        assert_eq!(sender.cycle(), 1 + i);
        assert_eq!(receiver.cycle(), 1 + i);
        assert_eq!(source_counter.load(Ordering::Relaxed), 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 1);
        assert_eq!(handled_counter.load(Ordering::Relaxed), 1);
    }

    while accumulator < ((period * 2) - 1)
    {
        accumulator += 1;

        let datagram = sender.poll_datagram(timestamp);
        assert!(datagram.is_none());
        assert_eq!(sender.cycle(), WINDOW_SIZE);
        assert_eq!(receiver.cycle(), WINDOW_SIZE);
        assert_eq!(source_counter.load(Ordering::Relaxed), 1);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 1);
        assert_eq!(handled_counter.load(Ordering::Relaxed), 1);
    }

    for i in 0..WINDOW_SIZE
    {
        let mut datagram = Box::new(*sender.poll_datagram(timestamp).unwrap());
        receiver.handle_datagram(timestamp, &mut datagram);
        assert_eq!(sender.cycle(), WINDOW_SIZE + 1 + i);
        assert_eq!(receiver.cycle(), WINDOW_SIZE + 1 + i);
        assert_eq!(source_counter.load(Ordering::Relaxed), 2);
        assert_eq!(sink_counter.load(Ordering::Relaxed), 2);
        assert_eq!(handled_counter.load(Ordering::Relaxed), 2);
    }
}
