//! An example guest mod: Rust compiled to wasm32-unknown-unknown (design doc, E7.4a).
//!
//! `api.rs` is the checked-in golden output of `efescript api-export --rust` for
//! `tests/fixtures/guest_test.efapi`. `run` touches every import on purpose, so the test can
//! check the whole import list of the built `.wasm`.

#![no_std]

extern crate alloc;

#[path = "../../guest_test.api.rs"]
mod api;

use alloc::vec::Vec;
use api::{engine, game, Score, Transform};
use efe_guest::{provide, provide_event, scratch, Entity};

scratch!(4096);

provide_event! {
    /// A coin was picked up.
    coin_picked: Score
}

provide_event! {
    /// The level restarted.
    level_restarted
}

provide! {
    /// Adds two numbers, saturating.
    fn add(a: i32, b: i32) -> i32 {
        a.saturating_add(b)
    }
}

provide! {
    /// Sums the bytes of `data`, the length of `name`, and the score and handle it is given.
    fn weigh(name: str, data: bytes, cfg: &Score, who: Entity, on: bool) -> f64 {
        let sum: u32 = data.iter().map(|b| *b as u32).sum();
        let bonus = if on { cfg.points } else { 0 };
        sum as f64 + name.len() as f64 + bonus as f64 + who.to_bits() as f64
    }
}

provide! {
    /// Traps when `n` is negative.
    fn checked(n: i32) -> i32 {
        if n < 0 {
            panic!("negative");
        }
        n
    }
}

provide! {
    /// Calls imports of the bindings.
    fn run(n: i32) -> i32 {
        let e = engine::world::spawn();
        let mut buf = [0u8; 20];
        let t = Transform::default();
        let mut status = unsafe { engine::world::get(e, Transform::COMPONENT as i32, buf.as_mut_ptr()) };
        status += engine::world::set_transform(e, &t);
        if engine::world::is_alive(e) {
            status += 1;
        }
        game::log("hello from rust");
        game::r#move(e, 1.0, 2.0, true);
        let score = Score::default();
        if game::blit(&[1, 2, 3], 3) {
            status += 1;
        }
        if game::r#match(n, "x", &[], &score) {
            status += 1;
        }
        status += game::scale(1.5) as i32;
        status += game::pointer(8) as usize as i32;
        game::r#async::spawn(e);
        game::score_changed::send(&score);
        status += game::score_changed::count() as i32;
        status += game::score_changed::iter().count() as i32;
        game::round_reset::send();
        status += game::round_reset::count() as i32;
        game::r#async::tick::send(&api::Wide::default());
        status += game::r#async::tick::count() as i32;
        status += game::r#async::tick::iter().count() as i32;
        let mut v: Vec<i32> = Vec::new();
        for i in 0..n {
            v.push(i);
        }
        status + v.len() as i32
    }
}
