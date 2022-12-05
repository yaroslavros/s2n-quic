use crate::prelude::*;

macro_rules! systems {
    ($($name:ident),* $(,)?) => {
        $(
            pub mod $name;
        )*

        pub fn setup<C: Config>(dispatch: &mut DispatcherBuilders, config: &mut C) {
            $(
                $name::setup(dispatch, config);
            )*
        }
    }
}

systems!(packet_rx_classify);
