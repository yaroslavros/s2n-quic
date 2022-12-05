use crate::prelude::{Dispatcher, DispatcherBuilder};

pub struct Dispatchers<'a, 'b> {
    pub rx: Dispatcher<'a, 'b>,
    pub tx: Dispatcher<'a, 'b>,
}

#[derive(Default)]
pub struct DispatcherBuilders<'a, 'b> {
    pub rx: DispatcherBuilder<'a, 'b>,
    pub tx: DispatcherBuilder<'a, 'b>,
}

impl<'a, 'b> DispatcherBuilders<'a, 'b> {
    pub fn build(self) -> Dispatchers<'a, 'b> {
        Dispatchers {
            rx: self.rx.build(),
            tx: self.tx.build(),
        }
    }
}
