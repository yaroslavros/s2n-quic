// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{error::Error, net::SocketAddr, path::Path};

pub use s2n_quic::provider::tls::s2n_tls;
use s2n_quic::{client::Connect, Client};

pub static CA_CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/ca-cert.pem");
pub static CLIENT_CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/client-cert.pem");
pub static CLIENT_KEY_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/client-key.pem");

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    google().await;
    //let tls = s2n_tls::Client::builder()
    //    .with_certificate(Path::new(CA_CERT_PEM))?
    //    .with_client_identity(Path::new(CLIENT_CERT_PEM), Path::new(CLIENT_KEY_PEM))?
    //    .build()?;
    //
    //let client = Client::builder()
    //    .with_tls(tls)?
    //    .with_io("0.0.0.0:0")?
    //    .start()?;
    //
    //let addr: SocketAddr = "127.0.0.1:4433".parse()?;
    //let connect = Connect::new(addr).with_server_name("localhost");
    //let mut connection = client.connect(connect).await?;
    //
    //// ensure the connection doesn't time out with inactivity
    //connection.keep_alive(true)?;
    //
    //// open a new stream and split the receiving and sending sides
    //let stream = connection.open_bidirectional_stream().await?;
    //let (mut receive_stream, mut send_stream) = stream.split();
    //
    //// spawn a task that copies responses from the server to stdout
    //tokio::spawn(async move {
    //    let mut stdout = tokio::io::stdout();
    //    let _ = tokio::io::copy(&mut receive_stream, &mut stdout).await;
    //});
    //
    //// copy data from stdin and send it to the server
    //let mut stdin = tokio::io::stdin();
    //tokio::io::copy(&mut stdin, &mut send_stream).await?;

    Ok(())
}

async fn google() {
    //let ca = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/mozilla-ca-bundle.pem"));
    //
    let mut config = s2n_tls::config::Builder::new();
    config
        // .trust_pem(ca)? // this works
        .with_system_certs(true)
        .unwrap() // doesn't work
        .set_application_protocol_preference([b"h3"])
        .unwrap()
        .set_security_policy(&s2n_tls::security::DEFAULT_TLS13)
        .unwrap()
        .enable_quic()
        .unwrap();

    // FIXME REMOVE to test with ca.
    // used to verify that it works without certs
    unsafe {
        config.disable_x509_verification().unwrap();
    }

    let config = config.build().unwrap();

    // FIXME which port does google use?
    // https://142.250.31.99/images/branding/googlelogo/2x/googlelogo_color_272x92dp.png
    //let socket_addr: SocketAddr = "142.250.31.99:4433".parse().unwrap();
    let socket_addr: SocketAddr = "142.250.31.99:443".parse().unwrap();
    let sni = "google.com";

    let tls = s2n_tls::Client::from_loader(config);
    let connect = s2n_quic::client::Connect::new(socket_addr).with_server_name(sni.to_owned());

    let client = s2n_quic::Client::builder()
        .with_tls(tls)
        .unwrap()
        .with_io("0.0.0.0:0")
        .unwrap()
        .start()
        .unwrap();

    let mut connection = client.connect(connect).await.unwrap();
    let stream = connection.open_bidirectional_stream().await.unwrap();
    let (mut _receive_stream, mut _send_stream) = stream.split();
}
