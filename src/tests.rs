#![feature(negative_impls)]
#![allow(unused)]
use std::{
    io::{self, Write},
    mem,
    ops::Add,
    time::{self, Duration, Instant},
};

use hashbrown::HashMap;

use crate::prelude::*;

#[derive(Debug, Component, PartialEq)]
pub struct DropTest {
    s: u32,
}
#[derive(Debug, Component, PartialEq)]
pub struct DropTest2 {
    s: u32,
}
#[derive(Debug, Component, PartialEq)]
pub struct DropTest3 {
    s: u32,
}
#[derive(Debug, Component, PartialEq)]
pub struct DropTest4 {
    s: u32,
}
#[derive(Debug, Component, PartialEq)]
pub struct DropTest5 {
    s: u32,
}

impl Drop for DropTest {
    fn drop(&mut self) {}
}

#[derive(Debug, Component, PartialEq)]
pub struct Tuple([usize; 3]);

#[derive(Debug, Resource)]
struct EntityCounter {
    map: HashMap<Entity, (usize, Duration)>,
}

async fn first_system(e: Entity, d: &mut DropTest, t: &mut DropTest2) {
    mem::swap(&mut d.s, &mut t.s);
}
async fn second_system(e: Entity, d: &mut DropTest3, t: &mut DropTest4) {
    mem::swap(&mut d.s, &mut t.s);
}
async fn third_system(e: Entity, d: &mut DropTest3, t: &mut DropTest5) {
    mem::swap(&mut d.s, &mut t.s);
}
#[derive(Component, Debug, PartialEq)]
pub struct Num(usize);
#[derive(Component, Debug, PartialEq)]
pub struct Str(String);
#[tokio::test(flavor = "multi_thread", worker_threads = 15)]
async fn graph_works() {
    let mut registry = Registry::default();
    let mut a = vec![10; 60000];
    let i = std::time::Instant::now();
    for i in 0..30000 {
        let a1 = a[2 * i];
        let b1 = a[2 * i + 1];
        a[2 * i] = b1;
        a[2 * i + 1] = a1;
    }
    println!("{:?}", std::time::Instant::now().duration_since(i));
    for i in 0..10000usize {
        let e = registry.spawn();
        registry.insert(e, (DropTest { s: i as u32 }, DropTest2 { s: i as u32 }));
    }
    for i in 0..10000usize {
        let e = registry.spawn();
        registry.insert(
            e,
            (
                DropTest { s: i as u32 },
                DropTest2 { s: i as u32 },
                DropTest3 { s: i as u32 },
            ),
        );
    }
    for i in 0..10000usize {
        let e = registry.spawn();
        registry.insert(
            e,
            (
                DropTest { s: i as u32 },
                DropTest2 { s: i as u32 },
                DropTest3 { s: i as u32 },
                DropTest4 { s: i as u32 },
            ),
        );
    }
    for i in 0..10000usize {
        let e = registry.spawn();
        registry.insert(
            e,
            (
                DropTest { s: i as u32 },
                DropTest2 { s: i as u32 },
                DropTest3 { s: i as u32 },
                DropTest5 { s: i as u32 },
            ),
        );
    }
    registry.create(EntityCounter {
        map: Default::default(),
    });

    // async fn first_system(e: Entity, d: &mut DropTest, t: &mut DropTest2) {
    //     mem::swap(&mut d.s, &mut t.s);
    // }
    // async fn second_system(e: Entity, d: &mut DropTest3, t: &mut DropTest4) {
    //     mem::swap(&mut d.s, &mut t.s);
    // }
    // async fn third_system(e: Entity, d: &mut DropTest3, t: &mut DropTest5) {
    //     mem::swap(&mut d.s, &mut t.s);
    // }
    let i = std::time::Instant::now();
    for (d, t) in <(&mut DropTest, &mut DropTest2)>::query(&mut registry) {
        mem::swap(&mut d.s, &mut t.s);
    }
    for (d, t) in <(&mut DropTest3, &mut DropTest4)>::query(&mut registry) {
        mem::swap(&mut d.s, &mut t.s);
    }
    for (d, t) in <(&mut DropTest3, &mut DropTest5)>::query(&mut registry) {
        mem::swap(&mut d.s, &mut t.s);
    }
    println!("{:?}", std::time::Instant::now().duration_since(i));
    let mut scheduler = Scheduler::default();
    scheduler.add_seq(first_system);
    scheduler.add_seq(second_system);
    scheduler.add_seq(third_system);

    let i = std::time::Instant::now();
    scheduler.execute(&mut registry).await;
    println!("{:?}", std::time::Instant::now().duration_since(i));
}
