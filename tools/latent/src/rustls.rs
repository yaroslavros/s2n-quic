use crate::{io::make_requests, stats::Stats, Error, Opts, Result, Server, CERT, KEY};
use rustls_pemfile as pem;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{
    rustls::{self, ClientConfig, RootCertStore, ServerConfig},
    TlsAcceptor, TlsConnector,
};

pub async fn client(opts: Opts) -> Result<()> {
    let mut root_store = RootCertStore::empty();
    root_store.add_parsable_certificates(&pem::certs(&mut std::io::Cursor::new(CERT))?);

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let client = TlsConnector::from(Arc::new(config));

    let addr: std::net::SocketAddr = opts.addr.parse()?;

    let stats = Stats::default();
    let connections = stats.recorders(opts.connections);
    stats.spawn(&opts);

    let mut scope = tokio::task::JoinSet::new();
    for rec in connections {
        let stream = TcpStream::connect(addr).await?;
        let conn = client.connect("localhost".try_into()?, stream).await?;

        make_requests(&mut scope, rec, opts.size, conn);
    }

    while scope.join_next().await.is_some() {}

    Ok(())
}

pub async fn server(opts: Server) {
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert(), pk())
        .unwrap();

    let server = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind(&opts.addr).await.unwrap();

    while let Ok((stream, _peer_addr)) = listener.accept().await {
        let server = server.clone();
        tokio::spawn(async move {
            let tls = server.accept(stream).await?;
            let (mut recv, mut send) = tokio::io::split(tls);
            tokio::io::copy(&mut recv, &mut send).await?;
            Ok::<_, Error>(())
        });
    }
}

fn cert() -> Vec<rustls::Certificate> {
    pem::certs(&mut std::io::Cursor::new(CERT))
        .unwrap()
        .into_iter()
        .map(rustls::Certificate)
        .collect()
}

fn pk() -> rustls::PrivateKey {
    rustls::PrivateKey(
        pem::pkcs8_private_keys(&mut std::io::Cursor::new(KEY))
            .unwrap()
            .pop()
            .unwrap(),
    )
}
