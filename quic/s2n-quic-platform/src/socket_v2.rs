// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use cfg_if::cfg_if;
use core::ops::ControlFlow;

pub trait EventHandler {
    fn on_complete(&mut self, count: usize) -> ControlFlow<(), ()>;

    fn on_error(&mut self, error: ::std::io::Error) -> ControlFlow<(), ()>;
}

pub mod task;

#[cfg(s2n_quic_platform_socket_msg)]
pub mod msg;

#[cfg(s2n_quic_platform_socket_mmsg)]
pub mod mmsg;

pub mod std;

cfg_if! {
    if #[cfg(s2n_quic_platform_socket_mmsg)] {
        pub use mmsg as default;
    } else if #[cfg(s2n_quic_platform_socket_msg)] {
        pub use msg as default;
    } else {
        pub use self::std as default;
    }
}
