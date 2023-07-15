use core::{arch, slice};
use std::{
    any::{self, TypeId},
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr,
    sync::Arc,
};

mod tests;

pub mod prelude {
    pub use crate::*;
}

use hashbrown::HashMap;

pub trait Component: 'static + Send + Sync {
    fn id(&self) -> TypeId;
    fn size(&self) -> usize;
}

impl<T: 'static + Send + Sync> Component for T {
    fn id(&self) -> TypeId {
        any::TypeId::of::<Self>()
    }
    fn size(&self) -> usize {
        mem::size_of::<Self>()
    }
}

pub type Identifier = usize;
pub type Generation = usize;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub struct Entity(Identifier, Generation);

pub struct Entities {
    cursor: Box<dyn Iterator<Item = Identifier> + 'static + Send + Sync>,
    free: Vec<Entity>,
}

impl Default for Entities {
    fn default() -> Self {
        Self {
            cursor: Box::new(0..),
            free: vec![],
        }
    }
}

impl Entities {
    pub fn spawn(&mut self) -> Entity {
        if let Some(entity) = self.free.pop() {
            entity
        } else {
            Entity(self.cursor.next().unwrap(), 0)
        }
    }

    pub fn despawn(&mut self, entity: Entity) {
        let Entity(identifier, generation) = entity;
        self.free.push(Entity(identifier, generation + 1));
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub struct ArchetypeInfo {
    ty: TypeId,
    size: usize,
}

#[derive(Default, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Archetype {
    len: usize,
    info: [Option<ArchetypeInfo>; 32],
}

impl Archetype {
    fn translation(&self, ty: TypeId) -> usize {
        let (idx, _) = self
            .info
            .iter()
            .take(self.len)
            .cloned()
            .map(Option::unwrap)
            .enumerate()
            .map(|(i, x)| (i, x.ty))
            .find(|(_, x)| *x == ty)
            .unwrap();

        self.size_up_to(idx)
    }
    fn is_subset_of(&self, superset: Archetype) -> bool {
        let ids = self
            .info
            .iter()
            .take(self.len)
            .cloned()
            .map(Option::unwrap)
            .map(|x| x.ty)
            .collect::<Vec<_>>();
        superset
            .info
            .iter()
            .take(superset.len)
            .cloned()
            .map(Option::unwrap)
            .map(|x| x.ty)
            .all(|id| ids.contains(&id))
    }
    fn total_size(&self) -> usize {
        self.size_up_to(self.len)
    }

    fn size_up_to(&self, idx: usize) -> usize {
        self.info
            .iter()
            .cloned()
            .take(idx)
            .map(Option::unwrap)
            .fold(0, |sum, info| sum + info.size)
    }
}

pub struct Storage {
    archetype: Archetype,
    data: Vec<u8>,
    mapping: Vec<Entity>,
    secondary_mapping: HashMap<Entity, usize>,
    table: Vec<Vec<Box<dyn Component>>>,
}

pub struct Swap {
    from: usize,
    to: usize,
}

impl Storage {
    fn new(archetype: Archetype) -> Self {
        Self {
            archetype,
            data: vec![],
            mapping: vec![],
            secondary_mapping: HashMap::new(),
            table: vec![],
        }
    }
    fn push(&mut self, entity: Entity, data: Vec<u8>, repr: Vec<Box<dyn Component>>) {
        let individual_size = self.archetype.total_size();

        if individual_size == 0 {
            return;
        }

        debug_assert_eq!(data.len(), individual_size);

        let idx = self.data.len() / individual_size;
        self.data.extend(data);
        self.mapping.push(entity);
        self.secondary_mapping.insert(entity, idx);
        self.table.push(repr);
    }

    fn remove(&mut self, entity: Entity) -> (Vec<u8>, Vec<Box<dyn Component>>) {
        let individual_size = self.archetype.total_size();
        if individual_size == 0 {
            return (vec![], vec![]);
        }
        //TODO this operation can maybe be optimized
        let idx = self.secondary_mapping.remove(&entity).unwrap();
        self.mapping.swap_remove(idx);
        let table = self.table.swap_remove(idx);
        if self.mapping.len() != 0 {
            self.secondary_mapping.insert(self.mapping[idx], idx);
        }

        let original_len = self.data.len() / individual_size;

        let removed_data = self
            .data
            .drain(idx * individual_size..(idx + 1) * individual_size)
            .collect::<Vec<_>>();

        if idx != original_len - 1 {
            let from = original_len - 1;

            let end = self.data[(from - 1) * individual_size..]
                .iter()
                .cloned()
                .collect::<Vec<_>>();

            self.data
                .splice(idx * individual_size..idx * individual_size, end);
            self.data.resize(self.data.len() - individual_size, 0);
        }

        (removed_data, table)
    }
}

pub trait Queryable {
    fn add(archetype: &mut Archetype);
    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self;
}

impl<'a, T: Component> Queryable for &'a T {
    fn add(archetype: &mut Archetype) {
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
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T>(),
                size: mem::size_of::<T>(),
            },
        );

        archetype.len = trimmed_archetype.len();
        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }
    }

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self {
        unsafe { (ptr.add(translations[idx]) as *const T).as_ref().unwrap() }
    }
}

impl<'a, T: Component> Queryable for &'a mut T {
    fn add(archetype: &mut Archetype) {
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
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T>(),
                size: mem::size_of::<T>(),
            },
        );

        archetype.len = trimmed_archetype.len();
        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }
    }

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self {
        unsafe { (ptr.add(translations[idx]) as *mut T).as_mut().unwrap() }
    }
}

impl Queryable for Entity {
    fn add(archetype: &mut Archetype) {}

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self {
        unsafe { ptr::read(entity) }
    }
}

impl<A: Queryable, B: Queryable> Queryable for (A, B) {
    fn add(archetype: &mut Archetype) {
        A::add(archetype);
        B::add(archetype);
    }
    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self {
        (
            A::get(archetype, ptr, entity, translations, idx + 0),
            B::get(archetype, ptr, entity, translations, idx + 1),
        )
    }
}

impl<A: Queryable, B: Queryable, C: Queryable> Queryable for (A, B, C) {
    fn add(archetype: &mut Archetype) {
        A::add(archetype);
        B::add(archetype);
        C::add(archetype);
    }
    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self {
        (
            A::get(archetype, ptr, entity, translations, idx + 0),
            B::get(archetype, ptr, entity, translations, idx + 1),
            C::get(archetype, ptr, entity, translations, idx + 2),
        )
    }
}

impl<A: Queryable, B: Queryable, C: Queryable, D: Queryable> Queryable for (A, B, C, D) {
    fn add(archetype: &mut Archetype) {
        A::add(archetype);
        B::add(archetype);
        C::add(archetype);
        D::add(archetype)
    }
    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: usize,
    ) -> Self {
        (
            A::get(archetype, ptr, entity, translations, idx + 0),
            B::get(archetype, ptr, entity, translations, idx + 1),
            C::get(archetype, ptr, entity, translations, idx + 2),
            D::get(archetype, ptr, entity, translations, idx + 3),
        )
    }
}
pub struct Query<Q: Queryable + ?Sized> {
    query_archetype: Archetype,
    total_size: usize,
    mapping: Vec<(Archetype, usize, Vec<usize>, *mut u8, *const Entity)>,
    inner: usize,
    outer: usize,
    marker: PhantomData<Q>,
}

pub trait QueryExt: Queryable {
    fn query(registry: &mut Registry) -> Query<Self>;
}

impl<T: Queryable> QueryExt for T {
    fn query(registry: &mut Registry) -> Query<Self> {
        let mut query_archetype = Default::default();

        T::add(&mut query_archetype);

        let mut mapping = vec![];

        for (storage_archetype, storage) in &mut registry.storage {
            if !storage_archetype.is_subset_of(query_archetype) {
                continue;
            }

            let mut translations = vec![];

            for ty in query_archetype
                .info
                .iter()
                .take(query_archetype.len)
                .cloned()
                .map(Option::unwrap)
                .map(|x| x.ty)
            {
                translations.push(storage_archetype.translation(ty));
            }

            mapping.push((
                storage_archetype.clone(),
                storage.data.len() / storage_archetype.total_size(),
                translations,
                storage.data.as_mut_ptr(),
                storage.mapping.as_ptr(),
            ));
        }

        Query {
            total_size: query_archetype.total_size(),
            query_archetype,
            mapping,
            inner: 0,
            outer: 0,
            marker: PhantomData,
        }
    }
}

impl<Q: Queryable> Iterator for Query<Q> {
    type Item = Q;
    fn next(&mut self) -> Option<Self::Item> {
        if self.inner >= self.mapping[self.outer].1 {
            self.outer += 1;
            self.inner = 0;
        }
        if self.outer >= self.mapping.len() {
            None?
        }

        let (archetype, _, translations, data, entities) = &self.mapping[self.outer];

        let ptr = unsafe { data.add(self.total_size * self.inner) };
        let entity = unsafe { entities.add(self.inner) };

        self.inner += 1;

        Some(Q::get(archetype, ptr, entity, translations, 0))
    }
}

#[derive(Default)]
pub struct Registry {
    entities: Entities,
    mapping: HashMap<Entity, Archetype>,
    storage: HashMap<Archetype, Storage>,
}

impl Registry {
    pub fn spawn(&mut self) -> Entity {
        let e = self.entities.spawn();
        self.mapping.insert(e, Default::default());
        e
    }
    pub fn despawn(&mut self, entity: Entity) {
        self.entities.despawn(entity);

        let archetype = self.mapping.remove(&entity).unwrap();
        if archetype.len != 0 {
            self.storage.get_mut(&archetype).unwrap().remove(entity);
        }
    }
    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        let mut archetype = self.mapping.remove(&entity).unwrap();

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
            result.unwrap()
        } else {
            result.unwrap_err()
        };

        let bytes =
            unsafe { slice::from_raw_parts(&component as *const _ as *const u8, component.size()) }
                .to_vec();

        let size_up_to = archetype.size_up_to(idx);

        data.splice(size_up_to..size_up_to, bytes);
        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: component.id(),
                size: component.size(),
            },
        );
        table.insert(idx, Box::new(component));

        debug_assert!(archetype.len + 1 < 32, "too many components in storage");

        archetype.len = trimmed_archetype.len();
        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }

        self.storage
            .entry(archetype.clone())
            .or_insert(Storage::new(archetype.clone()))
            .push(entity, data, table);
        self.mapping.insert(entity, archetype);
    }
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let mut archetype = self.mapping.get(&entity).unwrap().clone();

        if archetype.len == 0 {
            None?;
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
            None?
        };

        self.mapping.remove(&entity);
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
}
