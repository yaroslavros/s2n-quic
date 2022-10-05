use crate::{stats::Recorder, Result};
use std::collections::VecDeque;
use tokio::{io, task};

pub fn make_requests<C: 'static + io::AsyncWrite + io::AsyncRead + Unpin + Send>(
    tasks: &mut task::JoinSet<()>,
    mut hist: Recorder,
    size: usize,
    mut conn: C,
) {
    use io::AsyncWriteExt;

    let task = async move {
        let mut send_buf = vec![42u8; size + 8];
        let mut recv_buf = MsgBuf::new(size);

        loop {
            let timestamp = hist.timestamp();
            send_buf[..8].copy_from_slice(&timestamp.to_be_bytes());
            conn.write_all(&send_buf).await?;
            conn.flush().await?;

            let mut timestamp = [0; 8];
            recv_buf.read_exact(&mut conn, &mut timestamp).await?;
            let timestamp = u64::from_be_bytes(timestamp);

            recv_buf.skip(&mut conn, size).await?;

            hist.record(timestamp);
        }
    };

    tasks.spawn(async move {
        let _: Result<()> = task.await;
    });
}

struct MsgBuf {
    buffer: VecDeque<u8>,
    len: usize,
}

impl MsgBuf {
    pub fn new(size: usize) -> Self {
        Self {
            buffer: vec![0; size + 8].into(),
            len: 0,
        }
    }

    pub async fn read_exact<T: io::AsyncRead + Unpin>(
        &mut self,
        conn: &mut T,
        dst: &mut [u8],
    ) -> io::Result<()> {
        while self.len < dst.len() {
            self.buffer(conn).await?;
        }

        for (from, to) in self.buffer.iter().zip(dst.iter_mut()) {
            *to = *from;
        }

        self.clear(dst.len());

        Ok(())
    }

    pub async fn skip<T: io::AsyncRead + Unpin>(
        &mut self,
        conn: &mut T,
        size: usize,
    ) -> io::Result<()> {
        let mut remaining = size;
        while remaining > 0 {
            if self.len > 0 {
                let amount = remaining.min(self.len);
                self.clear(amount);
                remaining -= amount;
            } else {
                self.buffer(conn).await?;
            }
        }

        Ok(())
    }

    async fn buffer<T: io::AsyncRead + Unpin>(&mut self, conn: &mut T) -> io::Result<()> {
        use io::AsyncReadExt;
        let dst = &mut self.buffer.make_contiguous()[self.len..];
        let len = conn.read(dst).await?;
        self.len += len;
        Ok(())
    }

    fn clear(&mut self, len: usize) {
        if len == 0 {
            return;
        }

        if self.len == len {
            self.len = 0;
            return;
        }

        self.buffer.rotate_left(len);
        self.len -= len;
    }
}
