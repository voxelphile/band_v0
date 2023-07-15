use std::time;

use crate::Registry;

pub struct DropTest {
    s: u32,
}

impl Drop for DropTest {
    fn drop(&mut self) {
        println!("drop test success!");
    }
}

#[test]
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

    for i in 0..10000usize {
        let e = registry.spawn();
        registry.insert(e, "Hello, e!".to_owned() + &i.to_string());
        registry.insert(e, i);
    }

    use crate::QueryExt;
    let mut timing = Vec::with_capacity(10000);
    let mut last = time::Instant::now();
    for (a, b) in <(&usize, &String)>::query(&mut registry) {
        let now = time::Instant::now();

        timing.push(now.duration_since(last).as_nanos());

        last = now;
    }

    let mut avg = timing.iter().cloned().fold(0, |sum, x| sum + x) as f64 / timing.len() as f64;

    dbg!(avg); //534 nanoseconds
}
