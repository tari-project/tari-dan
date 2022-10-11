use std::{convert::Infallible, time::Duration};

use async_trait::async_trait;
use cucumber::{given, then, when, WorldInit};
use tokio::time::sleep;

// These `Cat` definitions would normally be inside your project's code,
// not test code, but we create them here for the show case.
#[derive(Debug, Default)]
struct Cat {
    pub hungry: bool,
}

impl Cat {
    fn feed(&mut self) {
        self.hungry = false;
    }
}

// `World` is your shared, likely mutable state.
// Cucumber constructs it via `Default::default()` for each scenario.
#[derive(Debug, Default, WorldInit)]
pub struct AnimalWorld {
    cat: Cat,
}

#[async_trait(?Send)]
impl cucumber::World for AnimalWorld {
    type Error = Infallible;

    async fn new() -> Result<Self, Self::Error> {
        Ok(Self {
            cat: Cat { hungry: false },
        })
    }
}

// Steps are defined with `given`, `when` and `then` attributes.
#[given(expr = "a {word} cat")]
async fn hungry_cat(world: &mut AnimalWorld, state: String) {
    sleep(Duration::from_secs(2)).await;

    match state.as_str() {
        "hungry" => world.cat.hungry = true,
        "satiated" => world.cat.hungry = false,
        s => panic!("expected 'hungry' or 'satiated', found: {}", s),
    }
}

#[when("I feed the cat")]
async fn feed_cat(world: &mut AnimalWorld) {
    sleep(Duration::from_secs(2)).await;
    world.cat.feed();
}

#[then("the cat is not hungry")]
async fn cat_is_fed(world: &mut AnimalWorld) {
    sleep(Duration::from_secs(2)).await;
    assert!(!world.cat.hungry);
}

// This runs before everything else, so you can setup things here.
#[tokio::main]
async fn main() {
    // You may choose any executor you like (`tokio`, `async-std`, etc.).
    // You may even have an `async` main, it doesn't matter. The point is that
    // Cucumber is composable. :)
    futures::executor::block_on(AnimalWorld::run("tests/features/book"));
}
