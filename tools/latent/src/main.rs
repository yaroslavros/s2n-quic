use structopt::StructOpt;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Clone, Debug)]
pub struct Error(String);

impl<T: std::error::Error> From<T> for Error {
    fn from(t: T) -> Self {
        Self(t.to_string())
    }
}

type Result<T, E = Error> = core::result::Result<T, E>;

static KEY: &str = include_str!("../../../quic/s2n-quic-core/certs/key.pem");
static CERT: &str = include_str!("../../../quic/s2n-quic-core/certs/cert.pem");

mod io;
mod quic;
mod rustls;
mod stats;
mod tcp;
mod tls;

#[derive(Debug, StructOpt)]
enum Args {
    Quic(Opts),
    Tcp(Opts),
    Tls(Opts),
    Rustls(Opts),
    Server(Server),
}

impl Args {
    fn is_multi_thread(&self) -> bool {
        match self {
            Self::Quic(o) => o.multi_thread,
            Self::Tcp(o) => o.multi_thread,
            Self::Tls(o) => o.multi_thread,
            Self::Rustls(o) => o.multi_thread,
            Self::Server(o) => o.multi_thread,
        }
    }

    async fn run(self) -> Result<()> {
        match self {
            Args::Quic(opts) => quic::client(opts).await,
            Args::Tls(opts) => tls::client(opts).await,
            Args::Tcp(opts) => tcp::client(opts).await,
            Args::Rustls(opts) => rustls::client(opts).await,
            Args::Server(opts) => {
                let quic = quic::server(opts.clone());

                let tcp = opts.tcp.clone();
                let tcp = async move {
                    if let Some(tcp) = tcp.as_ref() {
                        tcp::server(tcp).await;
                    } else {
                        core::future::pending().await
                    }
                };

                if opts.rustls {
                    let tls = rustls::server(opts);
                    tokio::join!(quic, tls, tcp);
                } else {
                    let tls = tls::server(opts);
                    tokio::join!(quic, tls, tcp);
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub struct Opts {
    #[structopt(long, default_value = "600")]
    size: usize,

    #[structopt(long, default_value = "10")]
    connections: usize,

    #[structopt(long)]
    multiplex: bool,

    #[structopt(default_value = "127.0.0.1:4433")]
    addr: String,

    #[structopt(long)]
    multi_thread: bool,

    #[structopt(long)]
    time: Option<u64>,

    #[structopt(long)]
    tsv: bool,
}

#[derive(Clone, Debug, StructOpt)]
pub struct Server {
    #[structopt(default_value = "0.0.0.0:4433")]
    addr: String,

    #[structopt(long)]
    tcp: Option<String>,

    #[structopt(long)]
    rustls: bool,

    #[structopt(long)]
    multi_thread: bool,
}

impl Opts {}

fn main() -> Result<()> {
    let format = tracing_subscriber::fmt::format()
        .with_level(false) // don't include levels in formatted output
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_ansi(false)
        .compact(); // Use a less verbose output format.

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .event_format(format)
        .init();

    let args = Args::from_args();

    if args.is_multi_thread() {
        main_multi_thread(args)
    } else {
        main_current_thread(args)
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main_multi_thread(args: Args) -> Result<()> {
    args.run().await
}

#[tokio::main(flavor = "current_thread")]
async fn main_current_thread(args: Args) -> Result<()> {
    args.run().await
}
