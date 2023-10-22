#![feature(fn_traits, async_closure, async_fn_in_trait)]

use core::{arch, slice};
use std::{
    any::{self, TypeId},
    collections::BTreeMap,
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ops, ptr,
    sync::Arc,
    time::Duration,
};
use std::future::Future;
use std::ops::Mul;
use futures::stream::FuturesUnordered;
use futures::StreamExt;

mod tests;

pub mod prelude {
    pub use crate::*;
}

use hashbrown::{HashMap, HashSet};
use tokio::sync::mpsc::{Receiver, Sender, unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub trait Component: 'static + Send + Sync {
    fn id(&self) -> TypeId;
    fn size(&self) -> usize;
}

pub trait Resource: 'static + Send + Sync {
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

impl<T: 'static + Send + Sync> Resource for T {
    fn id(&self) -> TypeId {
        any::TypeId::of::<Self>()
    }
    fn size(&self) -> usize {
        mem::size_of::<Self>()
    }
}

pub type Identifier = usize;
pub type Generation = usize;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash, Debug)]
pub struct Entity(Identifier, Generation);

pub struct Entities {
    cursor: Box<dyn Iterator<Item = Identifier> + 'static + Send + Sync>,
    free: HashSet<Entity>,
    dead: HashSet<Entity>,
}

impl Default for Entities {
    fn default() -> Self {
        Self {
            cursor: Box::new(0..),
            free: HashSet::new(),
            dead: HashSet::new(),
        }
    }
}

impl Entities {
    pub fn spawn(&mut self) -> Entity {
        if let Some(entity) = self.free.iter().next().cloned() {
            self.free.remove(&entity);
            entity
        } else {
            Entity(self.cursor.next().unwrap(), 0)
        }
    }

    pub fn despawn(&mut self, entity: Entity) -> bool {
        if self.dead.contains(&entity) {
            return false;
        }
        self.dead.insert(entity);
        let Entity(identifier, generation) = entity;
        let entity = Entity(identifier, generation + 1);
        if self.free.contains(&entity) {
            false
        } else {
            self.free.insert(entity);
            true
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub enum ArchetypeState {
    Component,
    Optional,
    Without,
    Resource,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub struct ArchetypeInfo {
    ty: TypeId,
    size: usize,
    name: &'static str,
    state: ArchetypeState,
}

#[derive(Default, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Archetype {
    len: usize,
    info: [Option<ArchetypeInfo>; 32],
}

impl Archetype {
    fn translation(&self, ty: TypeId) -> Option<usize> {
        let Some((idx, _)) = self
            .info
            .iter()
            .take(self.len)
            .cloned()
            .map(Option::unwrap)
            .enumerate()
            .map(|(i, x)| (i, x.ty))
            .find(|(_, x)| *x == ty) else {
                return None;
            };

        Some(self.size_up_to(idx))
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
            .all(|info| {
                if info.state == ArchetypeState::Without {
                    !ids.contains(&info.ty)
                } else {
                    ids.contains(&info.ty) || !matches!(info.state, ArchetypeState::Component)
                }
            })
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

        assert_eq!(data.len(), individual_size);

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
        let original_len = self.data.len() / individual_size;
        //TODO this operation can maybe be optimized
        let idx = self.secondary_mapping.remove(&entity).unwrap();
        self.mapping.swap_remove(idx);
        let table = self.table.swap_remove(idx);
        if idx != original_len - 1 {
            self.secondary_mapping.insert(self.mapping[idx], idx);
        }

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

pub trait Queryable<T> {
    type Target;
    fn add(archetype: &mut Archetype);
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    );
    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: &mut usize,
    ) -> Self;
    fn refs(deps: &mut HashSet<TypeId>) {}
    fn muts(deps: &mut HashSet<TypeId>) {}
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Without<T>(PhantomData<T>);

impl Queryable<()> for () {
    type Target = Self;

    fn add(archetype: &mut Archetype) {}

    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
    }

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: &mut usize,
    ) -> Self {
    }
}

pub struct ComponentMarker;
pub struct EntityMarker;
pub struct ResourceMarker;
pub struct MultiMarker;
pub struct OptionalMarker;
pub struct WithoutMarker;

impl<'a, T: Component> Queryable<ComponentMarker> for Without<T> {
    type Target = ();
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
            panic!("?");
        } else {
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T>(),
                size: mem::size_of::<T>(),
                name: any::type_name::<T>(),
                state: ArchetypeState::Without,
            },
        );

        archetype.len = trimmed_archetype.len();
        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }
    }

    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
    }

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: &mut usize,
    ) -> Self {
        Without(PhantomData)
    }
}

impl<'a, A, T: Queryable<A> + 'static> Queryable<(OptionalMarker, A)> for Option<T> {
    type Target = Self;
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
            panic!("?");
        } else {
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T::Target>(),
                name: any::type_name::<T::Target>(),
                size: mem::size_of::<T::Target>(),
                state: ArchetypeState::Optional,
            },
        );

        archetype.len = trimmed_archetype.len();
        for (i, info) in trimmed_archetype.into_iter().enumerate() {
            archetype.info[i] = Some(info);
        }
    }

    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
        if let Some(x) = storage_archetype.translation(any::TypeId::of::<T::Target>()) {
            translations.push(x);
        }
    }

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: &mut usize,
    ) -> Self {
        if archetype
            .info
            .iter()
            .find(|info| {
                let Some(info) = info else {
                return false;
            };
                info.ty == any::TypeId::of::<T::Target>()
            })
            .is_some()
        {
            let c = Some(T::get(archetype, ptr, entity, translations, idx));
            c
        } else {
            None
        }
    }

    fn refs(deps: &mut HashSet<TypeId>) {
        T::refs(deps);
    }

    fn muts(deps: &mut HashSet<TypeId>) {
        T::muts(deps);
    }
}

impl<'a, T: Component> Queryable<ComponentMarker> for &'a T {
    type Target = T;
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
        translations.push(
            storage_archetype
                .translation(any::TypeId::of::<T>())
                .unwrap(),
        );
    }
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
            panic!("?");
        } else {
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T>(),
                size: mem::size_of::<T>(),
                name: any::type_name::<T>(),
                state: ArchetypeState::Component,
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
        idx: &mut usize,
    ) -> Self {
        let c = unsafe { (ptr.add(translations[*idx]) as *const T).as_ref().unwrap() };
        *idx += 1;
        c
    }

    fn refs(deps: &mut HashSet<TypeId>) {
        deps.insert(any::TypeId::of::<T>());
    }
}

impl<'a, T: Component> Queryable<ComponentMarker> for &'a mut T {
    type Target = T;
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
        translations.push(
            storage_archetype
                .translation(any::TypeId::of::<T>())
                .unwrap(),
        );
    }
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
            panic!("?");
        } else {
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T>(),
                size: mem::size_of::<T>(),
                name: any::type_name::<T>(),
                state: ArchetypeState::Component,
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
        idx: &mut usize,
    ) -> Self {
        let c = unsafe { (ptr.add(translations[*idx]) as *mut T).as_mut().unwrap() };
        *idx += 1;
        c
    }

    fn refs(deps: &mut HashSet<TypeId>) {
        deps.insert(any::TypeId::of::<T>());
    }

    fn muts(deps: &mut HashSet<TypeId>) {
        deps.insert(any::TypeId::of::<T>());
    }
}

impl<'a, T: Resource> Queryable<ResourceMarker> for &'a T {
    type Target = T;
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
        translations.push(
            storage_archetype
                .translation(any::TypeId::of::<T>())
                .unwrap(),
        );
    }
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
            panic!("?");
        } else {
            result.unwrap_err()
        };

        trimmed_archetype.insert(
            idx,
            ArchetypeInfo {
                ty: any::TypeId::of::<T>(),
                size: mem::size_of::<T>(),
                name: any::type_name::<T>(),
                state: ArchetypeState::Resource,
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
        idx: &mut usize,
    ) -> Self {
        let c = unsafe { (ptr.add(translations[*idx]) as *const T).as_ref().unwrap() };
        *idx += 1;
        c
    }

    fn refs(deps: &mut HashSet<TypeId>) {
        deps.insert(any::TypeId::of::<T>());
    }
}

impl Queryable<EntityMarker> for Entity {
    type Target = Self;
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
    }
    fn add(archetype: &mut Archetype) {}

    fn get(
        archetype: &Archetype,
        ptr: *mut u8,
        entity: *const Entity,
        translations: &[usize],
        idx: &mut usize,
    ) -> Self {
        unsafe { ptr::read(entity) }
    }
}


pub struct Query<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> {
    mapping: Vec<(Archetype, usize, usize, Vec<usize>, *mut u8, *const Entity)>,
    inner: usize,
    outer: usize,
    marker: PhantomData<&'a Q>,
}

band_proc_macro::make_tuples!(16);

unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Send for Query<'a, Q, QQ> {}
unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Sync for Query<'a, Q, QQ> {}

pub trait QueryExt<QQ>: Queryable<QQ> {
    fn query(registry: &mut Registry) -> Query<Self, QQ>;
}

impl<'a, QQ: ?Sized, T: Queryable<QQ>> QueryExt<QQ> for T {
    fn query(registry: &mut Registry) -> Query<Self, QQ> {
        let mut query_archetype = Default::default();

        T::add(&mut query_archetype);

        let mut mapping = vec![];

        for (storage_archetype, storage) in &mut registry.storage {
            if storage.data.len() == 0 {
                continue;
            }

            if !storage_archetype.is_subset_of(query_archetype.clone()) {
                continue;
            }

            let mut translations = vec![];

            T::translations(&mut translations, &storage_archetype, &query_archetype);

            mapping.push((
                storage_archetype.clone(),
                storage.data.len() / storage_archetype.total_size(),
                storage_archetype.total_size(),
                translations,
                storage.data.as_mut_ptr(),
                storage.mapping.as_ptr(),
            ));
        }

        Query {
            mapping,
            inner: 0,
            outer: 0,
            marker: PhantomData,
        }
    }
}

impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Iterator for Query<'a, Q, QQ> {
    type Item = Q;
    fn next(&mut self) -> Option<Self::Item> {
        if self.mapping.len() == 0 {
            None?
        }
        if self.outer >= self.mapping.len() {
            None?
        }
        if self.inner >= self.mapping[self.outer].1 {
            self.outer += 1;
            self.inner = 0;
        }
        if self.outer >= self.mapping.len() {
            None?
        }

        let (archetype, _, total_size, translations, data, entities) = &self.mapping[self.outer];

        let ptr = unsafe { data.add(total_size * self.inner) };
        let entity = unsafe { entities.add(self.inner) };

        self.inner += 1;

        Some(Q::get(archetype, ptr, entity, translations, &mut 0))
    }
}

pub trait System<T, QQ> {
    type Param: Queryable<QQ>;
    fn execute(&self, registry: *mut Registry, param: Self::Param);
    fn ref_deps(&self) -> HashSet<TypeId>;
    fn mut_deps(&self) -> HashSet<TypeId>;
}

pub struct WrapperSystem<'a, QQ: ?Sized, T: Queryable<QQ>> {
    sub_system: Box<dyn System<T, QQ: ?Sized, Param = T>>,
    marker: PhantomData<&'a T>,
}

pub struct SubSystem<QQ: ?Sized, T: Queryable<QQ> + Send>(*mut Registry, *const dyn System<T, QQ: ?Sized, Param = T>);

impl<QQ: ?Sized, T: Queryable<QQ> + Send> Clone for SubSystem<QQ: ?Sized, T> {
    fn clone(&self) -> Self {
        Self(self.0, self.1)
    }
}

unsafe impl<QQ: ?Sized, T: Queryable<QQ> + Send> Send for SubSystem<QQ: ?Sized, T> {}
unsafe impl<QQ: ?Sized, T: Queryable<QQ> + Send> Sync for SubSystem<QQ: ?Sized, T> {}

impl<'a, QQ: ?Sized, A: Queryable<QQ> + Send> System<(), ()> for WrapperSystem<'a, QQ: ?Sized, A> {
    type Param = ();
    fn execute(&self, registry: *mut Registry, _: Self::Param) {
        let sub_system = SubSystem::<QQ: ?Sized, A>(registry, &*self.sub_system as *const _);
        for param in A::query(unsafe { registry.as_mut().unwrap() }) {
            let SubSystem(registry, sub_system) = sub_system.clone();
            unsafe { sub_system.as_ref().unwrap().execute(registry, param) };
        }
    }
    fn ref_deps(&self) -> HashSet<TypeId> {
        let mut deps = Default::default();
        A::refs(&mut deps);
        deps
    }
    fn mut_deps(&self) -> HashSet<TypeId> {
        let mut deps = Default::default();
        A::muts(&mut deps);
        deps
    }
}

band_proc_macro::make_systems!(16);

pub type NodeId = usize;
pub type Node = Box<dyn System<(), (), Param = ()>>;
#[derive(Default)]
pub struct Graph {
    nodes: Vec<Node>,
    dependencies: HashMap<NodeId, HashSet<NodeId>>,
}

impl Graph {
    fn add(&mut self, node: Node) {
        let my_id = self.nodes.len();

        self.nodes.push(node);

        let me = &self.nodes[my_id];

        let my_refs = me.ref_deps();
        let my_muts = me.mut_deps();

        let mut dependencies = HashSet::new();

        for (their_id, them) in self.nodes[..my_id].iter().enumerate().rev() {
            let their_refs = them.ref_deps();
            let their_muts = them.mut_deps();

            for my_ref in &my_refs {
                if their_muts.contains(my_ref) {
                    dependencies.insert(their_id);
                }
            }

            for my_mut in &my_muts {
                if their_refs.contains(my_mut) || their_muts.contains(my_mut) {
                    dependencies.insert(their_id);
                }
            }
        }

        self.dependencies.insert(my_id, dependencies);
    }
}

pub struct Scheduler {
    graph: Graph,
    work_tx: UnboundedSender<WorkUnit>,
    work_rx: UnboundedReceiver<WorkUnit>,
}

pub struct WorkUnit(NodeId, *mut Registry, *mut dyn System<(), (), Param = ()>);

unsafe impl Send for WorkUnit {}
unsafe impl Sync for WorkUnit {}

impl Scheduler {
    pub fn new(workers: usize) -> Self {
        let (work_tx, work_rx) = unbounded_channel();

        Self {
            graph: Default::default(),
            work_tx,
            work_rx,
        }
    }
    fn add<AA, A: Queryable<AA> + 'static + Send, T: System<A, AA, Param = A> + 'static>(&mut self, system: T) {
        let sub_system = Box::new(system);
        let wrapper_system = Box::new(WrapperSystem {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }
    async fn execute(&self, registry: &mut Registry) {
        let mut executing = Vec::new();

        let mut nodes_iter = self.graph.nodes.iter().enumerate();
        let mut futures = FuturesUnordered::default();
        loop {
            let Some((id, node)) = nodes_iter.next() else {
                break;
            };

            let finished = |executing: &[NodeId]| -> bool {
                for id in executing {
                    if self.graph.dependencies[id].contains(id) {
                        return false;
                    }
                }
                true
            };

            loop {
                if (finished)(&executing) {
                    break;
                }
                while let Some(Ok(id)) = futures.next().await {
                    executing.remove(id);
                }
            }

            executing.push(id);

            fn work_executor(work_unit: WorkUnit) -> impl FnOnce() -> NodeId + Send + 'static {
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(work_unit);
                move || -> NodeId {
                    let WorkUnit(id, registry, system) = rx.blocking_recv().expect("?");

                    unsafe {
                        system.as_ref().unwrap().execute(registry, ());
                    }

                    id
                }
            }

            let work_unit = WorkUnit(id, registry, (&**node) as *const _ as *mut _);

            let handle: JoinHandle<NodeId> = tokio::task::spawn_blocking(work_executor(work_unit));

            futures.push(handle);
        }
    }
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

    pub fn remove_all<T: Component, F: Fn(&mut Self, Entity) -> bool>(&mut self, predicate: F) {
        let mut remove_entities = HashSet::new();
        for entity in <(Entity, &T)>::query(self)
            .map(|(e, _)| e)
            .collect::<Vec<_>>()
        {
            if !(predicate)(self, entity) {
                continue;
            }
            remove_entities.insert(entity);
        }
        for entity in remove_entities {
            self.remove::<T>(entity);
        }
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
        let Some(translation) =  archetype.translation(any::TypeId::of::<T>()) else {
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
        let Some(translation) =  archetype.translation(any::TypeId::of::<T>()) else {
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
