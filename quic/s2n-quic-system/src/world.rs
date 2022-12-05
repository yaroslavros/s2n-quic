use crate::prelude::*;

pub fn new() -> World {
    let mut world = World::new();

    //crate::component::setup(&mut world);
    //crate::resource::setup(&mut world);

    world
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_test() {
        let _ = new();
    }
}
