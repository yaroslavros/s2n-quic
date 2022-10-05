use crate::{io::make_requests, stats::Stats, Error, Opts, Result};
use tokio::net::{TcpListener, TcpStream};

pub async fn server(addr: &str) {
    let listener = TcpListener::bind(addr).await.unwrap();

    while let Ok((stream, _peer_addr)) = listener.accept().await {
        tokio::spawn(async move {
            let (mut recv, mut send) = tokio::io::split(stream);
            tokio::io::copy(&mut recv, &mut send).await?;
            Ok::<_, Error>(())
        });
    }
}

pub async fn client(opts: Opts) -> Result<()> {
    let addr: std::net::SocketAddr = opts.addr.parse()?;

    let stats = Stats::default();
    let connections = stats.recorders(opts.connections);
    stats.spawn(&opts);

    let mut scope = tokio::task::JoinSet::new();
    for rec in connections {
        let stream = TcpStream::connect(addr).await?;
        make_requests(&mut scope, rec, opts.size, stream);
    }

    while scope.join_next().await.is_some() {}

    Ok(())
}
