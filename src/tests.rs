use std::time;

use hashbrown::HashMap;

use crate::{Registry, Entity, Without};

pub struct DropTest {
    s: u32,
}

impl Drop for DropTest {
    fn drop(&mut self) {
        println!("drop test success!");
    }
}

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
        registry.insert(e, Tuple([i, i+5, i+27]));
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

    
    for (entity, _,  Tuple([i1, i2, i3]), string) in <(Entity, Without<usize>, &Tuple, Option<&String>,)>::query(&mut registry){
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
        registry.insert(e, Tuple([i, i+5, i+27]));
        if i % 10 == 0 {
            registry.insert(e, i);
        }
        mapping.insert(e, i);
    }
    for (entity, _,  Tuple([i1, i2, i3]), string) in <(Entity, Without<usize>, &Tuple, Option<&String>,)>::query(&mut registry){
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
