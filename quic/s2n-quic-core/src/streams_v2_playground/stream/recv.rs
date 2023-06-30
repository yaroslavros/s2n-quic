use super::*;
use crate::sync::spsc;

#[cfg(test)]
mod tests;

const UNKNOWN_FINAL_SIZE: u64 = u64::MAX;

const UNKNOWN_RESET_CODE: u64 = u64::MAX;

pub struct Producer {
    state: SharedState,
    chunks: spsc::Producer<BytesMut>,
    cached_max_data: u64,
    cached_final_len: u64,
    is_reset: bool,
    // TODO make this a rbtree intrusive map
}

impl Producer {
    #[inline]
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            chunks: todo!(),
            cached_max_data: 0,
            cached_final_len: UNKNOWN_FINAL_SIZE,
            is_reset: false,
        }
    }

    #[inline]
    pub fn write_at(
        &mut self,
        offset: VarInt,
        data: &[u8],
        is_fin: bool,
    ) -> Result<(), transport::Error> {
        let max_offset = offset.as_u64().saturating_add(data.len() as u64);
        self.validate_write(max_offset)?;

        if is_fin {
            self.set_final_len(max_offset)?;
        }

        let buffer = unsafe { &mut *self.state.buffer.get() };

        if is_fin {
            buffer.write_at_fin(offset, data)
        } else {
            buffer.write_at(offset, data)
        }
        .map_err(|_| {
            transport::Error::FRAME_ENCODING_ERROR
                .with_reason("STREAM frame data would exceed the maximum VarInt")
        })?;

        Ok(())
    }

    #[inline]
    pub fn reset(&mut self, error_code: VarInt) {
        if core::mem::replace(&mut self.is_reset, true) {
            return;
        }

        self.state
            .reset_code
            .store(error_code.as_u64(), Ordering::Release);
    }

    #[inline]
    fn set_final_len(&mut self, final_len: u64) -> Result<(), transport::Error> {
        let prev = self.state.final_len.swap(final_len, Ordering::Release);

        let mut is_valid = false;

        // the final_offset is valid if it wasn't previously known
        is_valid |= prev == UNKNOWN_FINAL_SIZE;

        // or the previously known offset equals the current one
        is_valid |= prev == final_len;

        if !is_valid {
            return Err(transport::Error::FINAL_SIZE_ERROR);
        }

        self.cached_final_len = final_len;

        Ok(())
    }

    #[inline]
    pub fn needs_flush(&self) -> bool {
        self.is_reset || unsafe { !(*self.state.buffer.get()).is_empty() }
    }

    #[inline]
    pub fn flush(&mut self) {
        let buffer = unsafe { &mut *self.state.buffer.get() };

        let mut needs_wake = self.is_reset;

        let count = self.chunks.push_iter(buffer.drain());
        needs_wake |= count > 0;

        if needs_wake {
            // Wake up the consumer since we have new data
            self.state.consumer_waker.wake();
        }
    }

    #[inline]
    fn validate_write(&mut self, offset: u64) -> Result<(), transport::Error> {
        if self.cached_final_len != UNKNOWN_FINAL_SIZE && offset > self.cached_final_len {
            return Err(transport::Error::FINAL_SIZE_ERROR);
        }

        for i in 0..2 {
            if self.cached_max_data >= offset {
                return Ok(());
            }

            if i == 0 {
                self.cached_max_data = self.state.max_data.load(Ordering::Acquire);
            }
        }

        Err(transport::Error::STREAM_LIMIT_ERROR)
    }
}

pub struct StreamOps {
    pending_ops: Cursors<State>,
    connection_max_data: AtomicU64,
    connection_waker: Waker,
}

impl StreamOps {
    #[inline]
    pub fn new(connection_waker: Waker) -> Arc<Self> {
        unsafe {
            let ops = Self {
                pending_ops: Cursors::new(),
                connection_max_data: AtomicU64::new(0),
                connection_waker,
            };
            let ops = Arc::new(ops);
            ops.pending_ops.init();
            ops
        }
    }

    #[inline]
    fn push(&self, state: &SharedState) {
        // TODO
        let _ = state;
        self.connection_waker.wake_by_ref();
    }
}

pub struct Consumer {
    state: SharedState,
    chunks: spsc::Consumer<BytesMut>,
    stream_ops: Arc<StreamOps>,
    consumed_len: u64,
    cached_final_len: u64,
    last_flow_flush: u64,
    target_flow_window: u64,
    target_watermark: u64,
    buffer: Option<BytesMut>,
}

const SPIN_COUNT: usize = 4;

impl Consumer {
    #[inline]
    pub fn new(state: SharedState, stream_ops: Arc<StreamOps>) -> Self {
        Self {
            state,
            chunks: todo!(),
            stream_ops,
            consumed_len: 0,
            cached_final_len: UNKNOWN_FINAL_SIZE,
            last_flow_flush: 0,
            target_flow_window: 0,
            target_watermark: 0,
            buffer: None,
        }
    }

    pub fn pop(&mut self) {
        todo!()
    }

    #[inline]
    fn poll_chunks(&mut self, cx: &mut Context) -> Poll<BytesMut> {
        for i in 0..2 {
            if let Some(chunk) = self.chunks.pop() {
                return Poll::Ready(chunk);
            }

            if i == 0 {
                self.state.consumer_waker.register(cx.waker());
            }
        }

        Poll::Pending
    }

    #[inline]
    fn flush_max_data(&mut self) {
        // TODO take into account the final len

        let new_max_data = self.consumed_len.saturating_add(self.target_flow_window);
        let flow_watermark = self.last_flow_flush.saturating_add(self.target_watermark);

        // we haven't reached the watermark so don't update the max_data value
        if flow_watermark > new_max_data {
            return;
        }

        self.last_flow_flush = new_max_data;

        let prev_max_data = self
            .state
            .max_data
            .fetch_max(new_max_data, Ordering::Release);

        // check the previous value in case the `target_watermark` or `target_flow_window` changed
        if prev_max_data >= new_max_data {
            return;
        }

        // we're already in the queue to transmit so just wait our turn
        if !self.state.next.load(Ordering::Acquire).is_null() {
            return;
        }

        self.stream_ops.push(&self.state);
    }

    // TODO pop
    // TODO stop sending
    // TODO
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StreamOp;

/*
impl StreamOp {
    #[inline]
    pub fn stop_sending(&self) -> Option<frame::StopSending> {
        let application_error_code = self.0.stop_sending.load(Ordering::Acquire);
        let application_error_code = VarInt::new(application_error_code).ok()?;

        Some(frame::StopSending {
            stream_id: self.0.id.into(),
            application_error_code,
        })
    }

    #[inline]
    pub fn max_stream_data(&self) -> frame::MaxStreamData {
        let max_data = self.0.max_data.load(Ordering::Acquire);
        let maximum_stream_data =
            unsafe { VarInt::new_unchecked(max_data.min(VarInt::MAX.as_u64())) };

        frame::MaxStreamData {
            stream_id: self.0.id.into(),
            maximum_stream_data,
        }
    }
}
*/

pub struct State {
    id: stream::StreamId,
    buffer: UnsafeCell<ReceiveBuffer>,
    final_len: AtomicU64,
    max_data: AtomicU64,
    stop_sending: AtomicU64,
    reset_code: AtomicU64,
    consumer_waker: AtomicWaker,
    next: AtomicPtr<Self>,
}

#[derive(Clone)]
pub struct SharedState(Arc<UnsafeCell<State>>);

impl SharedState {
    pub fn new(state: State) -> Self {
        Self(Arc::new(UnsafeCell::new(state)))
    }
}

impl core::ops::Deref for SharedState {
    type Target = State;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0.get() }
    }
}

impl State {
    #[inline]
    fn new(id: stream::StreamId, initial_flow_window: VarInt) -> Self {
        Self {
            id,
            buffer: UnsafeCell::new(ReceiveBuffer::new()),
            final_len: AtomicU64::new(UNKNOWN_FINAL_SIZE),
            max_data: AtomicU64::new(initial_flow_window.as_u64()),
            stop_sending: AtomicU64::new(UNKNOWN_RESET_CODE),
            reset_code: AtomicU64::new(UNKNOWN_RESET_CODE),
            consumer_waker: AtomicWaker::new(),
            next: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    #[inline]
    fn init(&mut self, id: stream::StreamId, initial_flow_window: VarInt) {
        self.id = id;
        self.max_data
            .store(initial_flow_window.as_u64(), Ordering::Relaxed);
    }

    #[inline]
    fn clear(&mut self) {
        self.id = stream::StreamId::from_varint(VarInt::ZERO);
        self.buffer.get_mut().reset();
        self.final_len.store(UNKNOWN_FINAL_SIZE, Ordering::Relaxed);
        self.stop_sending
            .store(UNKNOWN_RESET_CODE, Ordering::Relaxed);
        self.reset_code.store(UNKNOWN_RESET_CODE, Ordering::Relaxed);
        let _ = self.consumer_waker.take();
        self.next.store(core::ptr::null_mut(), Ordering::Relaxed);
    }
}

impl mpsc::Entry for State {
    #[inline]
    unsafe fn placeholder() -> UnsafeCell<Self> {
        let state = State::new(stream::StreamId::from_varint(VarInt::ZERO), VarInt::ZERO);
        UnsafeCell::new(state)
    }

    #[inline]
    fn next(&self) -> &AtomicPtr<Self> {
        &self.next
    }

    #[inline]
    unsafe fn drop_ptr(ptr: NonNull<Self>) {
        let _ = Arc::from_raw(ptr.as_ptr());
    }
}
