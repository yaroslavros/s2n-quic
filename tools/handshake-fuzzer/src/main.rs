use core::time::Duration;
use rand::prelude::*;
use s2n_quic::{provider::event, Client, Server};
use s2n_quic_core::stream::testing::Data;

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
type Result<T = (), E = Error> = core::result::Result<T, E>;

fn main() -> Result {
    let format = tracing_subscriber::fmt::format()
        .with_level(false) // don't include levels in formatted output
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_ansi(false)
        .compact(); // Use a less verbose output format.

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .event_format(format)
        .init();

    let mut args = std::env::args();
    let _ = args.next();
    let arg = args.next();
    run(arg.as_deref())
}

#[tokio::main]
async fn run(arg: Option<&str>) -> Result {
    match arg {
        Some("server") => server().await,
        Some("client") => client().await,
        _ => Err("memory-report server|client".into()),
    }
}

async fn sleep_rand() {
    let ms = thread_rng().gen_range(0..50);
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

async fn client() -> Result {
    let io = ("0.0.0.0", 0);

    let tls = s2n_quic_core::crypto::tls::testing::certificates::CERT_PEM;

    let client = Client::builder()
        .with_io(io)?
        .with_tls(tls)?
        .with_event(event::tracing::Subscriber::default())?
        .start()
        .unwrap();

    loop {
        sleep_rand().await;

        let connect = s2n_quic::client::Connect::new(("127.0.0.1".parse()?, 4433))
            .with_server_name("localhost");
        let connect = client.connect(connect);

        tokio::spawn(async move {
            let connection = connect.await?;
            let stream_count = thread_rng().gen_range(0..10);

            for _ in 0..stream_count {
                sleep_rand().await;

                let mut handle = connection.handle();

                tokio::spawn(async move {
                    let stream = handle.open_bidirectional_stream();
                    let mut stream = stream.await?;

                    sleep_rand().await;

                    let mut data = Data::new(thread_rng().gen_range(0..2000));

                    while let Some(chunk) = data.send_one(usize::MAX) {
                        stream.send(chunk).await?;
                    }

                    stream.finish()?;

                    while stream.receive().await?.is_some() {}

                    Ok::<_, Error>(())
                });
            }

            Ok::<_, Error>(())
        });
    }
}

async fn server() -> Result {
    let io = ("127.0.0.1", 4433);

    let tls = (
        s2n_quic_core::crypto::tls::testing::certificates::CERT_PEM,
        s2n_quic_core::crypto::tls::testing::certificates::KEY_PEM,
    );

    let mut server = Server::builder()
        .with_io(io)?
        .with_tls(tls)?
        .with_event((event::tracing::Subscriber::default(), Subscriber))?
        .start()
        .unwrap();

    eprintln!("Server listening on port {}", io.1);

    while let Some(mut connection) = server.accept().await {
        tokio::spawn(async move {
            while let Ok(Some(mut stream)) = connection.accept_bidirectional_stream().await {
                tokio::spawn(async move {
                    while stream.receive().await?.is_some() {}

                    let mut data = Data::new(thread_rng().gen_range(0..200_000));

                    while let Some(chunk) = data.send_one(usize::MAX) {
                        stream.send(chunk).await?;
                    }

                    stream.finish()?;

                    Ok::<_, Error>(())
                });
            }
        });
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Subscriber;

#[derive(Debug, PartialEq)]
enum ConnState {
    Init(String, u64),
    Ready(String, u64),
    Sending,
}

impl ConnState {
    fn on_ready(&mut self) {
        match self {
            Self::Init(v, id) => *self = Self::Ready(core::mem::take(v), *id),
            _ => {}
        }
    }

    fn on_close(&mut self, evt: &event::events::ConnectionClosed) {
        if let Self::Ready(v, id) = self {
            println!("{}", v);
            panic!("closing in invalid state id({id}): {evt:?}");
        }
    }

    fn on_event<T: core::fmt::Debug>(&mut self, evt: &T) {
        match self {
            Self::Init(v, _) => *v += &format!("{evt:?}\n"),
            Self::Ready(v, _) => *v += &format!("{evt:?}\n"),
            Self::Sending => {}
        }
    }
}

impl event::Subscriber for Subscriber {
    type ConnectionContext = ConnState;

    #[inline]
    fn create_connection_context(
        &mut self,
        meta: &event::ConnectionMeta,
        _info: &event::ConnectionInfo,
    ) -> Self::ConnectionContext {
        ConnState::Init(Default::default(), meta.id)
    }

    #[inline]
    fn on_server_name_information(
        &mut self,
        ctx: &mut Self::ConnectionContext,
        _meta: &event::ConnectionMeta,
        _evt: &event::events::ServerNameInformation,
    ) {
        ctx.on_ready();
    }

    #[inline]
    fn on_packet_sent(
        &mut self,
        ctx: &mut Self::ConnectionContext,
        _meta: &event::ConnectionMeta,
        _evt: &event::events::PacketSent,
    ) {
        *ctx = ConnState::Sending;
    }

    #[inline]
    fn on_connection_closed(
        &mut self,
        ctx: &mut Self::ConnectionContext,
        _meta: &event::ConnectionMeta,
        evt: &event::events::ConnectionClosed,
    ) {
        ctx.on_close(evt);
    }

    #[inline]
    fn on_connection_event<E: event::Event>(
        &mut self,
        ctx: &mut Self::ConnectionContext,
        _meta: &event::ConnectionMeta,
        evt: &E,
    ) {
        ctx.on_event(evt);
    }
}
