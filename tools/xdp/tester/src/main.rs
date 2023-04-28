// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use aya::{
    maps::{HashMap, MapData, XskMap},
    programs::Xdp,
    Bpf,
};
use aya_log::BpfLogger;
use clap::Parser;
use core::sync::atomic::{AtomicU64, Ordering};
use log::{info, warn};
use s2n_codec::{DecoderBufferMut, Encoder, EncoderBuffer};
use s2n_quic_core::{
    inet::{ethernet::MacAddress, ipv4::IpV4Address},
    sync::{spsc, worker},
    xdp::{decoder::decode_packet, encoder, path},
};
use s2n_quic_xdp::{
    if_xdp::{Address, RxTxDescriptor, UmemDescriptor, XdpFlags},
    ring::{Completion, Fill, Rx, Tx},
    socket::Fd,
    syscall, umem,
};
use std::{os::fd::AsRawFd, sync::Arc};
use tokio::signal;

#[derive(Debug, Parser)]
struct Opt {
    /// The interface to run the program on
    #[clap(short, long, default_value = "lo")]
    iface: String,

    /// Traces BPF events
    #[clap(long)]
    trace: bool,

    /// Exits after attaching the BPF program
    ///
    /// This can be used to validate the correctness of the BPF program.
    #[clap(long)]
    exit_on_load: bool,
}

#[tokio::main(flavor = "current_thread")]
//#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let opt = Opt::parse();

    env_logger::init();

    let bpf = if opt.trace {
        s2n_quic_xdp::bpf::DEFAULT_PROGRAM_TRACE
    } else {
        s2n_quic_xdp::bpf::DEFAULT_PROGRAM
    };

    let mut bpf = Bpf::load(bpf)?;

    if opt.trace {
        if let Err(e) = BpfLogger::init(&mut bpf) {
            warn!("failed to initialize eBPF logger: {}", e);
        }
    }

    let program: &mut Xdp = bpf
        .program_mut(s2n_quic_xdp::bpf::PROGRAM_NAME)
        .unwrap()
        .try_into()?;
    program.load()?;

    if opt.exit_on_load {
        return Ok(());
    }

    program.attach(&opt.iface, Default::default())
        .context("failed to attach the XDP program with default flags - try changing XdpFlags::default() to XdpFlags::SKB_MODE")?;

    let mut ports: HashMap<_, _, _> = bpf
        .map_mut(s2n_quic_xdp::bpf::PORT_MAP_NAME)
        .expect("missing port map")
        .try_into()?;
    ports.insert(3000u16, 1u8, 0)?;

    let mut xskmap: XskMap<_> = bpf
        .map_mut(s2n_quic_xdp::bpf::XSK_MAP_NAME)
        .expect("missing sockets map")
        .try_into()?;
    listen(&opt.iface, &mut xskmap).map_err(|e| dbg!(e))?;

    info!("Waiting for Ctrl-C...");
    signal::ctrl_c().await?;
    info!("Exiting...");

    Ok(())
}

fn listen(iface: &str, xskmap: &mut XskMap<&mut MapData>) -> anyhow::Result<()> {
    let rx_queue_len = 1024;
    let tx_queue_len = 1024;
    let frame_size = 4096;
    let umem_size = rx_queue_len + tx_queue_len;

    let umem = umem::Builder {
        frame_count: umem_size,
        frame_size,
        ..Default::default()
    }
    .build()?;

    let socket_umem = Fd::open()?;
    socket_umem.attach_umem(&umem)?;

    let offsets_umem = syscall::offsets(&socket_umem)?;
    let fill = Fill::new(socket_umem.clone(), &offsets_umem, rx_queue_len * 2)?;
    let comp = Completion::new(socket_umem.clone(), &offsets_umem, tx_queue_len)?;

    // split for tx/rx

    let socket_tx = socket_umem.clone();
    let socket_rx = socket_umem;
    //let socket_tx = Fd::open()?;
    //let socket_rx = Fd::open()?;

    let offsets_tx = syscall::offsets(&socket_tx)?;
    let offsets_rx = syscall::offsets(&socket_rx)?;

    let rx = Rx::new(socket_rx.clone(), &offsets_rx, rx_queue_len)?;
    let tx = Tx::new(socket_tx.clone(), &offsets_tx, tx_queue_len * 2)?;

    let queue_id = 0;

    let mut addr = Address {
        queue_id,
        flags: XdpFlags::USE_NEED_WAKEUP,
        ..Default::default()
    };
    addr.set_if_name(&std::ffi::CString::new(iface)?)?;

    // addr.shared_umem_fd = socket_umem.as_raw_fd() as _;

    syscall::bind(&socket_tx, &mut addr)?;
    if socket_tx != socket_rx {
        syscall::bind(&socket_rx, &mut addr)?;
    }

    let mut desc = umem.frames();

    let (worker_send, worker_recv) = worker::channel();

    let rx_fill_queue = {
        let (mut send, recv) = spsc::channel((rx_queue_len * 2) as usize);

        let mut items = (&mut desc).take(rx_queue_len as _).peekable();

        while items.peek().is_some() {
            send.slice().extend(&mut items).unwrap();
        }

        tokio::spawn(s2n_quic_xdp::task::rx_to_fill(vec![recv], fill, ()));

        send
    };

    let tx_comp_queue = {
        let (mut send, recv) = spsc::channel((tx_queue_len * 2) as usize);

        let mut items = desc.peekable();

        while items.peek().is_some() {
            send.slice().extend(&mut items).unwrap();
        }

        tokio::spawn(s2n_quic_xdp::task::completion_to_tx(
            worker_recv,
            comp,
            frame_size,
            vec![send],
        ));

        recv
    };

    let reporter = Arc::new(Reporter::default());

    xskmap.set(queue_id, socket_rx.as_raw_fd(), 0)?;

    rx_tasks(rx, umem.clone(), socket_rx, rx_fill_queue, reporter.clone());
    tx_tasks(
        tx,
        umem,
        socket_tx,
        tx_comp_queue,
        worker_send,
        reporter.clone(),
    );

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(core::time::Duration::from_secs(1)).await;
            reporter.report();
        }
    });

    Ok(())
}

fn rx_tasks(
    rx: Rx,
    umem: umem::Umem,
    socket: Fd,
    done_send: spsc::Sender<UmemDescriptor>,
    reporter: Arc<Reporter>,
) {
    let (ready_send, ready_recv) = spsc::channel(1024 * 3);
    let fd = tokio::io::unix::AsyncFd::new(socket).unwrap();
    tokio::spawn(s2n_quic_xdp::task::rx(fd, rx, ready_send));
    rx_task_parse(ready_recv, done_send, umem, reporter);
}

#[allow(dead_code)]
fn rx_task_parse(
    mut ready: spsc::Receiver<RxTxDescriptor>,
    mut done: spsc::Sender<UmemDescriptor>,
    umem: umem::Umem,
    reporter: Arc<Reporter>,
) {
    let should_parse = std::env::var("SHOULD_PARSE").is_ok();

    tokio::spawn(async move {
        while ready.acquire().await.is_ok() {
            let mut rx_bytes = 0;
            let mut count = 0;
            let mut good_packets = 0;
            let mut bad_packets = 0;

            let mut ready = ready.slice();
            let (head, tail) = ready.peek();
            let mut rx_done = done.slice();

            for desc in head.iter().chain(tail.iter()).copied() {
                let packet = unsafe { umem.get_mut(desc) };

                count += 1;
                rx_bytes += packet.len();

                if should_parse {
                    let buffer = DecoderBufferMut::new(packet);

                    if decode_packet(buffer).map_or(false, |v| v.is_some()) {
                        good_packets += 1;
                    } else {
                        bad_packets += 1;
                    }
                }

                rx_done
                    .push(desc.into())
                    .expect("expected capacity in done queue");
            }

            ready.release(count);

            reporter.rx(count as _, rx_bytes as _, good_packets, bad_packets);
        }
    });
}

fn tx_tasks(
    tx: Tx,
    umem: umem::Umem,
    socket: Fd,
    mut comp: spsc::Receiver<UmemDescriptor>,
    worker: worker::Sender,
    reporter: Arc<Reporter>,
) {
    let mut buffer = [0u8; 3742];
    {
        let mut state = encoder::State::default();
        state.set_checksum(true);
        let mut buf = EncoderBuffer::new(&mut buffer);

        let mut message = message::Message {
            path: path::Tuple {
                local_address: path::LocalAddress {
                    mac: MacAddress::new([0x3c, 0x7c, 0x3f, 0x81, 0x7a, 0x7b]),
                    ip: IpV4Address::new([10, 0, 0, 10]).into(),
                    port: 3000,
                },
                remote_address: path::RemoteAddress {
                    mac: MacAddress::new([0x3c, 0x7c, 0x3f, 0x81, 0x7a, 0x7c]),
                    ip: IpV4Address::new([10, 0, 0, 11]).into(),
                    port: 3000,
                },
            },
            payload: vec![42u8; 3700],
        };

        encoder::encode_packet(&mut buf, &mut message, &mut state).unwrap();
        let encoded_len = buf.len();

        assert_eq!(buffer.len(), encoded_len);
    }

    let (mut tx_send, tx_recv) = spsc::channel::<RxTxDescriptor>(1024 * 3);

    let notifier = (socket, worker);
    tokio::spawn(s2n_quic_xdp::task::tx(tx_recv, tx, notifier));

    let delay = std::env::var("TX_DELAY").map_or(0, |v| v.parse().unwrap());

    tokio::spawn(async move {
        std::future::pending::<()>().await;

        loop {
            if delay > 0 {
                tokio::time::sleep(core::time::Duration::from_micros(delay)).await;
            }

            let _ = tokio::join!(comp.acquire(), tx_send.acquire());

            let mut comp = comp.slice();
            let mut tx_send = tx_send.slice();

            let mut count = 0;
            let mut bytes = 0;
            while let Some(desc) = comp.pop() {
                // dbg!(desc.address / 4096);
                let payload = unsafe { umem.get_mut(desc) };
                let len = buffer.len();

                debug_assert!(payload.len() >= len);

                payload[..len].copy_from_slice(&buffer);

                tx_send.push(desc.with_len(len as _)).unwrap();

                count += 1;
                bytes += len;
            }

            if count > 0 {
                reporter.tx(count as _, bytes as _);
            }
        }
    });
}

#[derive(Default)]
struct Reporter {
    tx_packets: AtomicU64,
    tx_bytes: AtomicU64,
    rx_packets: AtomicU64,
    rx_bytes: AtomicU64,
    good: AtomicU64,
    bad: AtomicU64,
}

impl Reporter {
    fn tx(&self, packets: u64, bytes: u64) {
        self.tx_packets.fetch_add(packets, Ordering::Relaxed);
        self.tx_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn rx(&self, packets: u64, bytes: u64, good: u64, bad: u64) {
        self.rx_packets.fetch_add(packets, Ordering::Relaxed);
        self.rx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.good.fetch_add(good, Ordering::Relaxed);
        self.bad.fetch_add(bad, Ordering::Relaxed);
    }

    fn report(&self) {
        let tx_packets = self.tx_packets.swap(0, Ordering::Relaxed);
        let tx_bytes = self.tx_bytes.swap(0, Ordering::Relaxed);
        let tx_rate = tx_bytes as f64 / 125_000_000.0;

        let rx_packets = self.rx_packets.swap(0, Ordering::Relaxed);
        let rx_bytes = self.rx_bytes.swap(0, Ordering::Relaxed);
        let rx_rate = rx_bytes as f64 / 125_000_000.0;

        let good = self.good.swap(0, Ordering::Relaxed);
        let bad = self.bad.swap(0, Ordering::Relaxed);

        let loss = tx_packets as i64 - rx_packets as i64;

        eprintln!(
            "TX[pkts={tx_packets}, rate={tx_rate:.2}Gbps] RX[pkts={rx_packets}, rate={rx_rate:.2}Gbps, good={good}, bad={bad}] DIFF[packets={loss}]"
        );
    }
}

mod message {
    use s2n_quic_core::{
        inet::ExplicitCongestionNotification,
        io::tx::{self, PayloadBuffer},
        xdp::path,
    };

    #[derive(Debug)]
    pub struct Message {
        pub path: path::Tuple,
        pub payload: Vec<u8>,
    }

    impl tx::Message for Message {
        type Handle = path::Tuple;

        fn path_handle(&self) -> &Self::Handle {
            &self.path
        }

        fn ecn(&mut self) -> ExplicitCongestionNotification {
            Default::default()
        }

        fn delay(&mut self) -> core::time::Duration {
            Default::default()
        }

        fn ipv6_flow_label(&mut self) -> u32 {
            0
        }

        fn can_gso(&self, _: usize, _: usize) -> bool {
            true
        }

        fn write_payload(
            &mut self,
            mut buffer: PayloadBuffer,
            _gso_offset: usize,
        ) -> Result<usize, tx::Error> {
            buffer.write(&self.payload)
        }
    }
}
