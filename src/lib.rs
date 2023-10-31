#![feature(fn_traits, async_closure, let_chains)]
pub mod archetype;
pub mod entity;
pub mod query;
pub mod scheduler;
pub mod storage;
use archetype::*;
use core::slice;
use entity::*;
pub mod prelude {
    pub use crate::entity::Entity;
    pub use crate::query::ParallelQueryExt;
    pub use crate::query::QueryExt;
    pub use crate::scheduler::Scheduler;
    pub use crate::Registry;
    pub use crate::{Component, Resource};
    pub use band_proc_macro::{Component, Resource};
    pub use rayon::iter::ParallelIterator;
}
use std::{
    any::{self, TypeId},
    mem::{self, ManuallyDrop},
    ptr,
};
use storage::*;
mod tests;

use hashbrown::HashMap;

pub trait Component: 'static + Send + Sync {
    fn id(&self) -> TypeId;
    fn size(&self) -> usize;
}

pub trait Resource: 'static + Send {
    fn id(&self) -> TypeId;
    fn size(&self) -> usize;
}

#[derive(Default)]
pub struct Registry {
    entities: Entities,
    mapping: HashMap<Entity, Archetype>,
    storage: HashMap<Archetype, Storage>,
    resources: HashMap<TypeId, Vec<u8>>,
}

impl Registry {
    pub fn spawn(&mut self) -> Entity {
        let e = self.entities.spawn();
        self.mapping.insert(e, Default::default());
        e
    }
    pub fn despawn(&mut self, entity: Entity) {
        if self.entities.despawn(entity) {
            let Some(archetype) = self.mapping.remove(&entity) else {
                return;
            };

            if archetype.len != 0 {
                self.storage.get_mut(&archetype).unwrap().remove(entity);
            }
        }
    }
    pub fn create<T: Resource>(&mut self, resource: T) {
        let bytes =
            unsafe { slice::from_raw_parts(&resource as *const _ as *const u8, resource.size()) }
                .to_vec();

        self.resources.insert(resource.id(), bytes);

        let _ = ManuallyDrop::new(resource);
    }
    pub fn resource<'a, 'b, T: Resource>(&'a mut self) -> Option<&'b T> {
        let Some(bytes) = self.resources.get(&any::TypeId::of::<T>()) else {
            None?
        };

        unsafe { Some((bytes.as_ptr() as *const T).as_ref().unwrap()) }
    }

    pub fn resource_mut<'a, 'b, T: Resource>(&'a mut self) -> Option<&'b mut T> {
        let Some(bytes) = self.resources.get_mut(&any::TypeId::of::<T>()) else {
            None?
        };

        unsafe { Some((bytes.as_mut_ptr() as *mut T).as_mut().unwrap()) }
    }

    pub fn destroy<T: Resource>(&mut self) -> Option<T> {
        let Some(bytes) = self.resources.remove(&any::TypeId::of::<T>()) else {
            None?
        };

        Some(unsafe { (bytes.as_ptr() as *const _ as *const T).read() })
    }

    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        let Some(mut archetype) = self.mapping.remove(&entity) else {
            return;
        };

        if archetype.translation(component.id()).is_some() {
            self.mapping.insert(entity, archetype);
            return;
        }

        let (mut data, mut table) = if archetype.len != 0 {
            self.storage.get_mut(&archetype).unwrap().remove(entity)
        } else {
            (vec![], vec![])
        };

        let mut trimmed_archetype = archetype
            .info
            .iter()
            .cloned()
            .map(Option::unwrap)
            .map(|x| x)
            .take(archetype.len)
            .collect::<Vec<_>>();

        let mut trimmed_type_ids = trimmed_archetype
            .iter()
            .map(|x| &x.ty)
            .cloned()
            .collect::<Vec<_>>();

        let result = trimmed_type_ids.binary_search(&component.id());

        let idx = if result.is_ok() {
            panic!("?")
        } else {
            result.unwrap_err()
        };

        let bytes =
            unsafe { slice::from_raw_parts(&component as *const _ as *const u8, component.size()) }
                .to_vec();

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: component.id(),
                size: component.size(),
                name: any::type_name::<T>(),
                state: ArchetypeState::Component,
            },
        );
        debug_assert!(archetype.len + 1 < 32, "too many components in storage");

        archetype.len = trimmed_archetype.len();
        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }

        let size_up_to = archetype.size_up_to(idx);

        data.splice(size_up_to..size_up_to, bytes);

        table.insert(idx, Box::new(component));

        self.storage
            .entry(archetype.clone())
            .or_insert(Storage::new(archetype.clone()))
            .push(entity, data, table);
        self.mapping.insert(entity, archetype);
    }
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let Some(mut archetype) = self.mapping.remove(&entity) else {
            None?
        };

        if archetype.len == 0 {
            self.mapping.insert(entity, archetype);
            None?;
        }

        if archetype.translation(any::TypeId::of::<T>()).is_none() {
            self.mapping.insert(entity, archetype);
            None?
        }

        let mut trimmed_archetype = archetype
            .info
            .iter()
            .cloned()
            .map(Option::unwrap)
            .map(|x| x)
            .take(archetype.len)
            .collect::<Vec<_>>();

        let mut trimmed_type_ids = trimmed_archetype
            .iter()
            .map(|x| &x.ty)
            .cloned()
            .collect::<Vec<_>>();

        let result = trimmed_type_ids.binary_search(&any::TypeId::of::<T>());

        let idx = if result.is_ok() {
            result.unwrap()
        } else {
            self.mapping.insert(entity, archetype);
            None?
        };

        let (mut data, mut table) = self.storage.get_mut(&archetype).unwrap().remove(entity);

        let size_up_to = archetype.size_up_to(idx);
        let ret = data[size_up_to..size_up_to + mem::size_of::<T>()].to_vec();
        data.splice(size_up_to..size_up_to + mem::size_of::<T>(), []);
        let _ = ManuallyDrop::new(table.remove(idx));

        trimmed_archetype.remove(idx);

        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }

        archetype.len -= 1;

        self.storage
            .entry(archetype.clone())
            .or_insert(Storage::new(archetype.clone()))
            .push(entity, data, table);
        self.mapping.insert(entity, archetype);

        Some(unsafe { ptr::read::<T>(ret.as_ptr() as *const T) })
    }
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        let Some(archetype) = self.mapping.get(&entity).cloned() else {
            None?
        };

        if archetype.len == 0 {
            None?;
        }

        let storage = self.storage.get(&archetype).unwrap();

        let Some(index) = storage.secondary_mapping.get(&entity).cloned() else {
            None?
        };

        let total_size = storage.archetype.total_size();
        let Some(translation) = archetype.translation(any::TypeId::of::<T>()) else {
            None?
        };
        unsafe {
            let ptr = storage.data.as_ptr().add(total_size * index + translation);
            Some((ptr as *const T).as_ref().unwrap())
        }
    }
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let Some(archetype) = self.mapping.get(&entity).cloned() else {
            None?
        };

        if archetype.len == 0 {
            None?;
        }

        let storage = self.storage.get_mut(&archetype).unwrap();

        let Some(index) = storage.secondary_mapping.get(&entity).cloned() else {
            None?
        };

        let total_size = storage.archetype.total_size();
        let Some(translation) = archetype.translation(any::TypeId::of::<T>()) else {
            None?
        };
        unsafe {
            let ptr = storage
                .data
                .as_mut_ptr()
                .add(total_size * index + translation);
            Some((ptr as *mut T).as_mut().unwrap())
        }
    }
    pub fn dbg_print(&self, entity: Entity) {
        use std::io::Write;
        let Some(archetype) = self.mapping.get(&entity).cloned() else {
            println!("entity does not have an archetype mapping.");
            std::io::stdout().flush().unwrap();
            return;
        };
        println!("archetype len: {}", archetype.len);
        for (i, name) in archetype
            .info
            .iter()
            .filter_map(|x| *x)
            .map(|x| x.name)
            .enumerate()
        {
            println!("component {}: {}", i, name);
        }
        std::io::stdout().flush().unwrap();
    }
}
