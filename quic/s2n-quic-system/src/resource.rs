use crate::prelude::*;

macro_rules! resources {
    ($($name:ident),* $(,)?) => {
        $(
            pub mod $name;
        )*

        pub fn setup<C: Config>(world: &mut World, config: &mut C) {
            $(
                $name::setup(world, config);
            )*
        }
    }
}

resources!(connection_id, clock, crypto, random);
