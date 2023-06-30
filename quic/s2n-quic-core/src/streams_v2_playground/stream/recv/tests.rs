use super::*;

use futures_test::task::new_count_waker;

fn id(v: u32) -> stream::StreamId {
    stream::StreamId::from_varint(VarInt::from_u32(v))
}

#[test]
fn simple_test() {
    /*
    let (conn_waker, conn_wake_count) = new_count_waker();
    let (recv_waker, recv_wake_count) = new_count_waker();
    let mut recv_cx = Context::from_waker(&recv_waker);
    let state = State::new(id(1), VarInt::from_u16(16));
    let state = SharedState::new(state);
    let stream_ops = StreamOps::new(conn_waker);
    let mut producer = Producer::new(state.clone());
    let mut consumer = Consumer::new(state.clone(), stream_ops.clone());

    let mut pool = ChunkPool::new();

    dbg!(consumer.poll_chunks(&mut recv_cx));

    producer
        .write_at(VarInt::ZERO, b"hello world!", true)
        .unwrap();

    assert_eq!(recv_wake_count.get(), 0);
    assert!(producer.needs_flush());

    producer.flush(&mut pool);
    assert_eq!(recv_wake_count.get(), 1);

    // dbg!(consumer.poll_chunks(&mut recv_cx));

    // panic!();
    */
}
