use crate::{io::make_requests, stats::Stats, Error, Opts, Result, Server, CERT, KEY};
use s2n_tls::{config::Config, security::DEFAULT_TLS13};
use s2n_tls_tokio::*;
use tokio::net::{TcpListener, TcpStream};

pub async fn server(opts: Server) {
    let mut config = Config::builder();
    config.set_security_policy(&DEFAULT_TLS13).unwrap();
    config.load_pem(CERT.as_bytes(), KEY.as_bytes()).unwrap();

    let server = TlsAcceptor::new(config.build().unwrap());

    let listener = TcpListener::bind(&opts.addr).await.unwrap();

    while let Ok((stream, _peer_addr)) = listener.accept().await {
        let server = server.clone();
        tokio::spawn(async move {
            let mut tls = server.accept(stream).await?;
            tls.as_mut().prefer_low_latency()?;
            let (mut recv, mut send) = tokio::io::split(tls);
            tokio::io::copy(&mut recv, &mut send).await?;
            Ok::<_, Error>(())
        });
    }
}

pub async fn client(opts: Opts) -> Result<()> {
    let mut config = Config::builder();
    config.set_security_policy(&DEFAULT_TLS13)?;
    config.trust_pem(CERT.as_bytes())?;

    let client = TlsConnector::new(config.build()?);

    let addr: std::net::SocketAddr = opts.addr.parse()?;

    let stats = Stats::default();
    let connections = stats.recorders(opts.connections);
    stats.spawn(&opts);

    let mut scope = tokio::task::JoinSet::new();
    for rec in connections {
        let stream = TcpStream::connect(addr).await?;
        let mut conn = client.connect("localhost", stream).await?;

        conn.as_mut().prefer_low_latency()?;

        make_requests(&mut scope, rec, opts.size, conn);
    }

    while scope.join_next().await.is_some() {}

    Ok(())
}
