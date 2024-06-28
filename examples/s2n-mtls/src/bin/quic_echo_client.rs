// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use s2n_quic::provider::tls::s2n_tls;
use std::error::Error;

pub static CA_CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/ca-cert.pem");
pub static CLIENT_CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/client-cert.pem");
pub static CLIENT_KEY_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/client-key.pem");

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    google().await;
    Ok(())
}

async fn google() {
    let mut config = s2n_tls::config::Builder::new();
    config
        .set_application_protocol_preference([b"h3"])
        .unwrap()
        .set_security_policy(&s2n_tls::security::DEFAULT_TLS13)
        .unwrap()
        .enable_quic()
        .unwrap();

    // DOESN'T WORK: sanity check that not including any CA certs
    // config.with_system_certs(false).unwrap();
    //
    // WORKS: The root cert `../../google-com.pem` is also part of my hosts system cert
    config.with_system_certs(true).unwrap();
    //
    // WORKS
    // config
    //     .with_system_certs(false)
    //     .unwrap()
    //     .trust_pem(include_bytes!("../../google-com.pem"))
    //     .unwrap(); // this works

    // FIXME REMOVE to test with ca.
    // used to verify that it works without certs
    // unsafe {
    //     config.disable_x509_verification().unwrap();
    // }

    let config = config.build().unwrap();

    let sni = "google.com";
    // https://142.250.31.99/images/branding/googlelogo/2x/googlelogo_color_272x92dp.png
    // 443 seems to be the QUIC port. `4433` didn't work.
    let addr = tokio::net::lookup_host(format!("{sni}:443"))
        .await
        .unwrap()
        .next()
        .unwrap();
    println!("dns resolution: {:?}", addr);

    let tls = s2n_tls::Client::from_loader(config);
    let connect = s2n_quic::client::Connect::new(addr).with_server_name(sni.to_owned());

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
