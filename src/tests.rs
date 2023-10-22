use std::{io::{self, Write}, thread, time};

use hashbrown::HashMap;

use crate::*;

#[derive(Debug)]
pub struct DropTest {
    s: u32,
}

impl Drop for DropTest {
    fn drop(&mut self) {}
}

#[derive(Debug)]
pub struct Tuple([usize; 3]);

//#[test]
fn it_works() {
    let mut registry = Registry::default();

    let e1 = registry.spawn();

    registry.insert(e1, "Hello, e1!");

    let e2 = registry.spawn();

    registry.insert(e2, "Hello, e2!");

    registry.insert(e1, 2usize);

    registry.insert(e2, 3usize);

    registry.insert(e1, DropTest { s: 5 });

    registry.despawn(e1);

    assert_eq!(registry.remove::<&'static str>(e2).unwrap(), "Hello, e2!");
    assert_eq!(registry.remove::<usize>(e2).unwrap(), 3usize);
}

#[test]
fn it_works2() {
    let mut registry = Registry::default();
    let mut mapping = HashMap::new();
    for i in 0..1000usize {
        let e = registry.spawn();
        if i % 5 == 0 {
            registry.insert(e, "Hello, e!".to_owned() + &i.to_string());
        }
        registry.insert(e, Tuple([i, i + 5, i + 27]));
        if i % 10 == 0 {
            registry.insert(e, i);
        }
        mapping.insert(e, i);
    }

    use crate::QueryExt;
    let mut timing = Vec::with_capacity(10000);
    let mut last = time::Instant::now();
    for (a, b) in <(&usize, &String)>::query(&mut registry) {
        let now = time::Instant::now();

        timing.push(now.duration_since(last).as_nanos());

        last = now;
    }

    for (entity, _, Tuple([i1, i2, i3]), string) in
        <(Entity, Without<usize>, &Tuple, Option<&String>)>::query(&mut registry)
    {
        if let Some(s) = string {
            dbg!(s);
        } else {
            dbg!("none");
        }
        assert_eq!(i1, mapping.get(&entity).unwrap());
        assert_eq!(*i2, mapping.get(&entity).unwrap() + 5);
        assert_eq!(*i3, mapping.get(&entity).unwrap() + 27);
    }
    let mut delete = vec![];
    for (entity, i) in mapping.iter().step_by(7) {
        registry.despawn(*entity);
        delete.push(*entity);
    }
    for entity in delete {
        mapping.remove(&entity);
    }
    for i in 1000..2000usize {
        let e = registry.spawn();
        if i % 5 == 0 {
            registry.insert(e, "Hello, e!".to_owned() + &i.to_string());
        }
        registry.insert(e, Tuple([i, i + 5, i + 27]));
        if i % 10 == 0 {
            registry.insert(e, i);
        }
        mapping.insert(e, i);
    }
    for (entity, _, Tuple([i1, i2, i3]), string) in
        <(Entity, Without<usize>, &Tuple, Option<&String>)>::query(&mut registry)
    {
        if let Some(s) = string {
            dbg!(s);
        } else {
            dbg!("none");
        }
        assert_eq!(i1, mapping.get(&entity).unwrap());
        assert_eq!(*i2, mapping.get(&entity).unwrap() + 5);
        assert_eq!(*i3, mapping.get(&entity).unwrap() + 27);
    }

    let mut avg = timing.iter().cloned().fold(0, |sum, x| sum + x) as f64 / timing.len() as f64;

    dbg!(avg); //534 nanoseconds
}
fn first_system(e: Entity, d: &DropTest, t: &Tuple) {
    thread::sleep(Duration::from_millis(2));
}
fn second_system(e: Entity, d: &mut DropTest, t: &Tuple) {
    thread::sleep(Duration::from_millis(2));
}
fn third_system(e: Entity, d: &DropTest, t: &mut Tuple) {
    thread::sleep(Duration::from_millis(2));
}
fn forth_system(e: Entity, d: &mut DropTest, t: &mut Tuple) {
    thread::sleep(Duration::from_millis(2));
}
fn fifth_system(e: Entity, d: &DropTest, t: &Tuple) {
    thread::sleep(Duration::from_millis(2));
}
fn sixth_system(e: Entity, d: &DropTest, t: &Tuple) {
    thread::sleep(Duration::from_millis(2));
}
fn seventh_system(e: Entity, d: &mut DropTest, t: &mut Tuple) {
    tokio::time::sleep(Duration::from_millis(2));
}
#[tokio::test]
async fn graph_works() {
    let mut registry = Registry::default();
    for i in 0..10000usize {
        let e = registry.spawn();
        println!("{}", i);
        if i % 2 == 0 {
            registry.insert(e, DropTest { s: i as u32 });
        }
        if i % 5 == 0 {
            registry.insert(e, "Hello, e!".to_owned() + &i.to_string());
        }
        registry.insert(e, Tuple([i, i + 5, i + 27]));
        if i % 10 == 0 {
            registry.insert(e, i);
        }
    }
    let i = std::time::Instant::now();
    let mut scheduler = Scheduler::new(8);
    scheduler.add(first_system);
    scheduler.add(second_system);
    scheduler.add(third_syst em);
    scheduler.add(forth_system);
    scheduler.add(fifth_system);
    scheduler.add(sixth_system);
    scheduler.add(seventh_system);
    dbg!("start");
    scheduler.execute(&mut registry).await;
    dbg!(std::time::Instant::now().duration_since(i));
    dbg!("vs");
    let i = std::time::Instant::now();
    for (e, d, t) in <(Entity, &DropTest, &Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    for (e, d, t) in <(Entity, &mut DropTest, &Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    for (e, d, t) in <(Entity, &DropTest, &mut Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    for (e, d, t) in <(Entity, &mut DropTest, &mut Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    for (e, d, t) in <(Entity, &DropTest, &Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    for (e, d, t) in <(Entity, &DropTest, &Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    for (e, d, t) in <(Entity, &mut DropTest, &mut Tuple)>::query(&mut registry) {
        thread::sleep(Duration::from_millis(2));
    }
    dbg!(std::time::Instant::now().duration_since(i));
}
