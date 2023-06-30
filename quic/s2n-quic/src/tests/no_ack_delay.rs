// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn no_ack_delay_test() {
    let model = Model::default();
    test(model, |handle| {
        let limits = provider::limits::Limits::default().with_max_ack_delay(Duration::ZERO)?;

        let server = Server::builder()
            .with_io(handle.builder().build()?)?
            .with_tls(SERVER_CERTS)?
            .with_event(events())?
            .with_limits(limits)?
            .start()?;
        let addr = start_server(server)?;

        let client = Client::builder()
            .with_io(handle.builder().build()?)?
            .with_tls(certificates::CERT_PEM)?
            .with_event(events())?
            .with_limits(limits)?
            .start()?;

        start_client(client, addr, Data::new(1_000_000))?;

        Ok(())
    })
    .unwrap();

    panic!();
}
