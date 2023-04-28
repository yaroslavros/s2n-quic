// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::Result;
use aya::{
    maps::{HashMap, MapData, XskMap},
    programs, Bpf,
};
use s2n_quic::provider::io::{
    self,
    xdp::{
        bpf, encoder,
        if_xdp::{self, XdpFlags},
        io as xdp_io, ring, socket, spsc, syscall, task, umem, Provider,
    },
};
use s2n_quic_core::io::tx::TxExt;
use std::{ffi::CString, net::ToSocketAddrs, os::unix::io::AsRawFd};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Xdp {
    #[structopt(long, default_value = "lo")]
    interface: String,

    #[structopt(long, default_value = "2048")]
    tx_queue_len: u32,

    #[structopt(long, default_value = "2048")]
    rx_queue_len: u32,

    #[structopt(long, default_value = "4096")]
    frame_size: u32,

    #[structopt(long)]
    xdp_stats: bool,

    #[structopt(long)]
    bpf_trace: bool,

    #[structopt(long, default_value = "auto")]
    xdp_mode: XdpMode,

    #[structopt(long)]
    split_queues: bool,
}

#[derive(Clone, Copy, Debug)]
enum XdpMode {
    Auto,
    Skb,
    Drv,
    Hw,
}

impl core::str::FromStr for XdpMode {
    type Err = crate::Error;

    fn from_str(v: &str) -> Result<Self, Self::Err> {
        Ok(match v {
            "auto" => Self::Auto,
            "skb" => Self::Skb,
            "drv" | "driver" => Self::Drv,
            "hw" | "hardware" => Self::Hw,
            _ => return Err(format!("invalid xdp-mode: {v:?}").into()),
        })
    }
}

impl From<XdpMode> for programs::xdp::XdpFlags {
    fn from(mode: XdpMode) -> Self {
        match mode {
            XdpMode::Auto => Self::default(),
            XdpMode::Skb => Self::SKB_MODE,
            XdpMode::Drv => Self::DRV_MODE,
            XdpMode::Hw => Self::HW_MODE,
        }
    }
}

impl Xdp {
    fn setup(
        &self,
    ) -> Result<(
        umem::Umem,
        Vec<(
            spsc::Receiver<if_xdp::RxTxDescriptor>,
            spsc::Sender<if_xdp::UmemDescriptor>,
        )>,
        Vec<(u32, socket::Fd)>,
        Vec<(
            spsc::Receiver<if_xdp::UmemDescriptor>,
            spsc::Sender<if_xdp::RxTxDescriptor>,
        )>,
        xdp_io::driver::Driver,
    )> {
        let frame_size = self.frame_size;
        let rx_queue_len = self.rx_queue_len;
        let tx_queue_len = self.tx_queue_len;
        let fill_ring_len = rx_queue_len * 2;
        let completion_ring_len = tx_queue_len;

        let mut max_queues = syscall::max_queues(&self.interface);

        let umem_size = if self.split_queues {
            if max_queues % 2 == 0 {
                (rx_queue_len + tx_queue_len) * (max_queues / 2)
            } else {
                (rx_queue_len * (max_queues / 2 + 1)) + (tx_queue_len * (max_queues / 2))
            }
        } else {
            (rx_queue_len + tx_queue_len) * max_queues
        };

        let umem = umem::Builder {
            frame_count: umem_size,
            frame_size,
            ..Default::default()
        }
        .build()?;

        let mut address = if_xdp::Address {
            flags: XdpFlags::USE_NEED_WAKEUP,
            ..Default::default()
        };
        address.set_if_name(&CString::new(self.interface.clone())?)?;

        let mut shared_umem_fd = None;
        let mut tx = vec![];
        let mut tx_driver = xdp_io::driver::Driver::default();
        let mut rx = vec![];
        let mut rx_fds = vec![];

        let mut desc = umem.frames();

        let mut push_rx = true;
        let mut push_tx = false;

        for queue_id in 0..max_queues {
            if self.split_queues {
                push_rx = !push_rx;
                push_tx = !push_tx;
            } else {
                push_rx = true;
                push_tx = true;
            }

            let socket = socket::Fd::open()?;

            if let Some(fd) = shared_umem_fd {
                address.set_shared_umem(&fd);
            } else {
                socket.attach_umem(&umem)?;
                shared_umem_fd = Some(socket.as_raw_fd());
            }

            let async_fd = std::sync::Arc::new(tokio::io::unix::AsyncFd::new(socket.clone())?);

            let offsets = syscall::offsets(&socket)?;

            if push_rx {
                let (mut send_fill, recv_fill) = spsc::channel((fill_ring_len * 2) as usize);
                let (send_occupied, recv_occupied) = spsc::channel((fill_ring_len * 2) as usize);

                let fill_ring = ring::Fill::new(socket.clone(), &offsets, fill_ring_len)?;
                let rx_ring = ring::Rx::new(socket.clone(), &offsets, rx_queue_len)?;
                rx_fds.push((queue_id, socket.clone()));

                let mut items = (&mut desc).take(rx_queue_len as _);
                send_fill
                    .try_slice()
                    .unwrap()
                    .unwrap()
                    .extend(&mut items)
                    .unwrap();

                tokio::spawn(task::rx_to_fill(vec![recv_fill], fill_ring, ()));

                let fd = async_fd.clone();
                tokio::spawn(task::rx(fd, rx_ring, send_occupied));

                rx.push((recv_occupied, send_fill));
            } else {
                syscall::set_fill_ring_size(&socket, fill_ring_len)?;
            }

            if push_tx {
                let (mut send_free, recv_free) = spsc::channel((completion_ring_len * 2) as usize);
                let (send_occupied, recv_occupied) =
                    spsc::channel((completion_ring_len * 2) as usize);

                let completion_ring =
                    ring::Completion::new(socket.clone(), &offsets, completion_ring_len)?;
                let tx_ring = ring::Tx::new(socket.clone(), &offsets, tx_queue_len)?;

                let mut items = (&mut desc).take(tx_queue_len as _);
                send_free
                    .try_slice()
                    .unwrap()
                    .unwrap()
                    .extend(&mut items)
                    .unwrap();

                let comp_to_tx = task::completion_to_tx(
                    task::completion_to_tx::OnDemand,
                    completion_ring,
                    frame_size,
                    vec![send_free],
                );
                tx_driver.push(Box::pin(comp_to_tx));

                //let fd = async_fd.clone();
                let fd = socket.clone();
                tokio::spawn(task::tx(recv_occupied, tx_ring, fd));

                tx.push((recv_free, send_occupied));
            } else {
                syscall::set_completion_ring_size(&socket, completion_ring_len)?;
            }

            address.queue_id = queue_id;

            syscall::bind(&socket, &mut address)?;
        }

        assert!(desc.next().is_none());

        Ok((umem, rx, rx_fds, tx, tx_driver))
    }

    fn bpf_task(&self, port: u16, rx_fds: Vec<(u32, socket::Fd)>) -> Result<()> {
        let mut bpf = if self.bpf_trace {
            let mut bpf = Bpf::load(bpf::DEFAULT_PROGRAM_TRACE)?;

            if let Err(err) = aya_log::BpfLogger::init(&mut bpf) {
                eprint!("error initializing BPF trace: {err:?}");
            }

            bpf
        } else {
            Bpf::load(bpf::DEFAULT_PROGRAM)?
        };

        let interface = self.interface.clone();
        let xdp_stats = self.xdp_stats;
        let xdp_mode = self.xdp_mode.into();

        let bpf_task = async move {
            let program: &mut programs::Xdp = bpf
                .program_mut(bpf::PROGRAM_NAME)
                .expect("missing default program")
                .try_into()?;
            program.load()?;

            let link_id = program.attach(&interface, xdp_mode)?;

            let mut ports: HashMap<_, _, _> = bpf
                .map_mut(bpf::PORT_MAP_NAME)
                .expect("missing port map")
                .try_into()?;

            ports.insert(port, 1u8, 0)?;

            let mut xskmap: XskMap<&mut MapData> = bpf
                .map_mut(bpf::XSK_MAP_NAME)
                .expect("missing socket map")
                .try_into()?;

            for (queue_id, socket) in &rx_fds {
                xskmap.set(*queue_id, socket.as_raw_fd(), 0)?;
            }

            if xdp_stats {
                loop {
                    tokio::time::sleep(core::time::Duration::from_secs(1)).await;
                    for (queue_id, socket) in &rx_fds {
                        if let Ok(stats) = syscall::statistics(socket) {
                            println!("stats[{queue_id}]: {stats:?}");
                        }
                    }
                }
            }

            core::future::pending::<()>().await;

            let _ = bpf;
            let _ = link_id;
            let _ = rx_fds;

            Result::<(), crate::Error>::Ok(())
        };

        tokio::spawn(async move {
            if let Err(err) = bpf_task.await {
                dbg!(err);
            }
        });

        Ok(())
    }

    pub fn server(&self, port: u16) -> Result<impl io::Provider> {
        let (umem, rx, rx_fds, tx, tx_driver) = self.setup()?;

        self.bpf_task(port, rx_fds)?;

        let io_rx = xdp_io::rx::Rx::new(rx, umem.clone());

        let io_tx = {
            let encoder = encoder::Config::default();
            xdp_io::tx::Tx::new(tx, umem, encoder).with_driver(tx_driver)
        };

        let provider = Provider::builder()
            .with_rx(io_rx)
            .with_tx(io_tx)
            .with_frame_size(self.frame_size as _)?
            .build();

        Ok(provider)
    }

    pub fn client(&self, port: u16) -> Result<impl io::Provider> {
        let interface = CString::new(self.interface.clone())?;

        let addr = ("::", port).to_socket_addrs()?.next().unwrap();
        let udp_socket = socket::bind_udp(&interface, addr)?;
        let udp_socket = tokio::io::unix::AsyncFd::new(udp_socket)?;

        let (umem, rx, rx_fds, tx, tx_driver) = self.setup()?;

        self.bpf_task(port, rx_fds)?;

        let io_rx = xdp_io::rx::Rx::new(rx, umem.clone());

        let io_tx = {
            let encoder = encoder::Config::default();
            let tx = xdp_io::tx::Tx::new(tx, umem, encoder).with_driver(tx_driver);

            let udp_tx = {
                let tx_queue_len = self.tx_queue_len as usize;

                let (mut send_free, recv_free) = spsc::channel(tx_queue_len * 2);
                let (send_occupied, recv_occupied) = spsc::channel(tx_queue_len);

                {
                    let mut slice = send_free.slice();
                    for _ in 0..tx_queue_len {
                        slice
                            .push(socket::msg::Packet::new(self.frame_size as _))
                            .unwrap();
                    }
                }

                tokio::spawn(socket::task::TxFlush::new(
                    recv_occupied,
                    udp_socket,
                    send_free,
                ));

                let udp_channels = vec![(recv_free, send_occupied)];
                socket::task::Tx::<socket::msg::Packet>::new(udp_channels)
                    .with_handle_map(|handle: &io::xdp::PathHandle| handle.into())
            };

            let tx = tx.with_router(xdp_io::router::Router::default(), udp_tx);

            tx
        };

        let provider = Provider::builder()
            .with_rx(io_rx)
            .with_tx(io_tx)
            .with_frame_size(self.frame_size as _)?
            .build();

        Ok(provider)
    }
}
