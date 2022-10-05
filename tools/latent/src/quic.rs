use crate::{
    stats::{Recorder, Stats},
    Opts, Result, Server, CERT, KEY,
};
use bytes::{Buf, Bytes};
use s2n_quic::{provider::event, stream::ReceiveStream};
use std::collections::VecDeque;
use tokio::task;

pub async fn server(opts: Server) {
    let mut server = s2n_quic::Server::builder()
        .with_tls((CERT, KEY))
        .unwrap()
        .with_io(&*opts.addr)
        .unwrap()
        .with_event(event::tracing::Provider::default())
        .unwrap()
        .with_limits(limits())
        .unwrap()
        .start()
        .unwrap();

    while let Some(mut connection) = server.accept().await {
        tokio::spawn(async move {
            while let Ok(Some(stream)) = connection.accept_bidirectional_stream().await {
                tokio::spawn(async move {
                    let (mut recv, mut send) = stream.split();
                    let mut chunks = vec![Bytes::new(); 16];
                    while let Ok((count, is_open)) = recv.receive_vectored(&mut chunks).await {
                        if send.send_vectored(&mut chunks[..count]).await.is_err() {
                            break;
                        }

                        if !is_open {
                            break;
                        }
                    }
                });
            }
        });
    }
}

pub async fn client(opts: Opts) -> Result<()> {
    let client = s2n_quic::Client::builder()
        .with_tls(CERT)?
        .with_io("0.0.0.0:0")?
        .with_event(event::tracing::Provider::default())?
        .with_limits(limits())?
        .start()?;

    let addr: std::net::SocketAddr = opts.addr.parse()?;
    let connect = s2n_quic::client::Connect::new(addr).with_server_name("localhost");

    let stats = Stats::default();
    let connections = stats.recorders(opts.connections);
    stats.spawn(&opts);

    let mut scope = tokio::task::JoinSet::new();

    if opts.multiplex {
        let mut connection = client.connect(connect.clone()).await?;
        for rec in connections {
            let stream = connection.open_bidirectional_stream().await?;

            make_requests(&mut scope, rec, opts.size, stream);
        }
    } else {
        for rec in connections {
            let mut connection = client.connect(connect.clone()).await?;
            let stream = connection.open_bidirectional_stream().await?;

            make_requests(&mut scope, rec, opts.size, stream);
        }
    }

    while scope.join_next().await.is_some() {}

    Ok(())
}

fn make_requests(
    tasks: &mut task::JoinSet<()>,
    mut hist: Recorder,
    size: usize,
    conn: s2n_quic::stream::BidirectionalStream,
) {
    let task = async move {
        let (recv, mut send) = conn.split();
        let send_buf = Bytes::from(vec![42u8; size]);
        let mut recv_buf = RecvBuf::new(recv);

        loop {
            let timestamp = hist.timestamp();
            let mut data = [
                Bytes::copy_from_slice(&timestamp.to_be_bytes()),
                send_buf.clone(),
            ];
            send.send_vectored(&mut data).await?;

            let mut timestamp = [0u8; 8];
            recv_buf.read_exact(&mut timestamp).await?;
            let timestamp = u64::from_be_bytes(timestamp);
            recv_buf.skip(size).await?;

            hist.record(timestamp);
        }
    };

    tasks.spawn(async move {
        let _: Result<()> = task.await;
    });
}

#[derive(Debug)]
struct RecvBuf {
    chunks: VecDeque<Bytes>,
    occupied: usize,
    len: usize,
    stream: ReceiveStream,
    is_open: bool,
}

impl RecvBuf {
    pub fn new(stream: ReceiveStream) -> Self {
        Self {
            chunks: vec![Bytes::new(); 32].into(),
            occupied: 0,
            len: 0,
            stream,
            is_open: true,
        }
    }
}

impl RecvBuf {
    pub async fn read_exact(&mut self, out: &mut [u8]) -> Result<()> {
        while self.len < out.len() {
            self.buffer().await?;
        }

        s2n_quic_core::slice::vectored_copy(
            &self.chunks.make_contiguous()[..],
            &mut [&mut out[..]],
        );

        self.clear(out.len());

        Ok(())
    }

    pub async fn skip(&mut self, size: usize) -> Result<()> {
        let mut remaining = size;
        while remaining > 0 {
            if self.len > 0 {
                let amount = remaining.min(self.len);
                self.clear(amount);
                remaining -= amount;
            } else {
                self.buffer().await?;
            }
        }

        Ok(())
    }

    fn clear(&mut self, len: usize) {
        if len == 0 {
            return;
        }

        if self.len == len {
            self.len = 0;
            self.occupied = 0;
            for chunk in self.chunks.iter_mut() {
                *chunk = Bytes::new();
            }
            return;
        }

        let mut remaining = len;

        loop {
            let chunk = &mut self.chunks[0];
            let len = chunk.len();
            if len <= remaining {
                self.chunks.pop_front();
                self.chunks.push_back(Bytes::new());
                self.len -= len;
                self.occupied -= 1;
                remaining -= len;
            } else {
                chunk.advance(remaining);
                self.len -= remaining;
                break;
            }
        }
    }

    async fn buffer(&mut self) -> Result<()> {
        if !self.is_open {
            panic!("stream was closed");
        }

        if self.occupied == self.chunks.len() {
            let len = self.chunks.len() * 2;
            self.chunks.resize(len, Bytes::new());
        }

        let chunks = &mut self.chunks.make_contiguous()[self.occupied..];
        let (count, is_open) = self.stream.receive_vectored(chunks).await?;
        self.is_open = is_open;

        self.occupied += count;
        for chunk in &chunks[..count] {
            self.len += chunk.len();
        }

        Ok(())
    }
}

fn limits() -> s2n_quic::provider::limits::Limits {
    let data_window = u32::MAX as u64;

    s2n_quic::provider::limits::Limits::default()
        .with_data_window(data_window)
        .unwrap()
        .with_max_send_buffer_size(data_window.min(u32::MAX as _) as _)
        .unwrap()
        .with_bidirectional_local_data_window(data_window)
        .unwrap()
        .with_bidirectional_remote_data_window(data_window)
        .unwrap()
        .with_unidirectional_data_window(data_window)
        .unwrap()
}
