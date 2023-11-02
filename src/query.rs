use crate::{
    archetype::*, entity::Entity, Component, ComponentBundle, Registry, RegistryHandle, Resource,
};
use rayon::iter::*;
use std::{any, any::TypeId, collections::*, marker::PhantomData, mem, ptr};

pub unsafe trait QuerySync<QQ: ?Sized> {}
unsafe impl<'a, T: Component> QuerySync<ComponentMarker> for &'a T {}
unsafe impl<'a, T: Component> QuerySync<ComponentMarker> for &'a mut T {}
unsafe impl<'a, T: Resource> QuerySync<ResourceMarker> for &'a T {}

unsafe impl QuerySync<EntityMarker> for Entity {}

unsafe impl<T> QuerySync<T> for () {}

unsafe impl<T: QuerySync<ResourceMarker>> QuerySync<OptionalResourceMarker> for Option<T> {}
unsafe impl<T: QuerySync<ComponentMarker>> QuerySync<OptionalComponentMarker> for Option<T> {}
pub trait Queryable<T: ?Sized> {
    type Target;
    fn add(archetype: &mut Archetype);
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    );
    fn get(state: &mut QueryState) -> Self;
    fn refs(deps: &mut HashSet<TypeId>) {}
    fn muts(deps: &mut HashSet<TypeId>) {}
}

pub(crate) enum Command {
    Spawn(Vec<Box<dyn Component>>),
    Insert(Entity, Vec<Box<dyn Component>>),
}

#[derive(Clone)]
pub struct Commands {
    tx: std::sync::mpsc::Sender<Command>,
    handle: RegistryHandle,
}

impl Commands {
    fn spawn(&mut self, bundle: impl ComponentBundle) {
        self.tx
            .send(Command::Spawn(bundle.into_component_iter().collect()));
    }
    fn insert(&mut self, entity: Entity, bundle: impl ComponentBundle) {
        self.tx.send(Command::Insert(
            entity,
            bundle.into_component_iter().collect(),
        ));
    }
}

impl<'a, T> Queryable<T> for &'a mut Commands {
    type Target = Commands;
    fn add(archetype: &mut Archetype) {}
    fn get(state: &mut QueryState) -> Self {
        unsafe { mem::transmute::<&'_ mut Commands, &'a mut Commands>(state.commands) }
    }
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
    }
}

pub struct QueryState<'a> {
    commands: &'a mut Commands,
    registry: *mut Registry,
    archetype: &'a Archetype,
    ptr: *mut u8,
    entity: *const Entity,
    translations: &'a [usize],
    idx: &'a mut usize,
}

unsafe impl Send for QueryState<'_> {}
unsafe impl Sync for QueryState<'_> {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Without<T>(PhantomData<T>);

impl<T> Queryable<T> for () {
    type Target = Self;

    fn add(archetype: &mut Archetype) {}

    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
    }

    fn get(state: &mut QueryState) -> Self {}
}

pub struct ComponentMarker;
pub struct EntityMarker;
pub struct ResourceMarker;
pub struct MultiMarker;
pub struct OptionalComponentMarker;
pub struct OptionalResourceMarker;
pub struct WithoutMarker;

impl<'a, T: Component> Queryable<ComponentMarker> for Without<T>
where
    T: QuerySync<ComponentMarker>,
{
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

    fn get(state: &mut QueryState) -> Self {
        Without(PhantomData)
    }
}

impl<T: Queryable<ComponentMarker> + 'static> Queryable<OptionalComponentMarker> for Option<T>
where
    T: QuerySync<ComponentMarker>,
{
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

    fn get(state: &mut QueryState) -> Self {
        if state
            .archetype
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
            let c = Some(T::get(state));
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

impl<'a, T: Queryable<ResourceMarker> + 'static> Queryable<OptionalResourceMarker> for Option<T>
where
    <T as Queryable<ResourceMarker>>::Target: Resource,
    T: QuerySync<ResourceMarker>,
{
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

    fn get(state: &mut QueryState) -> Self {
        unsafe {
            (&mut state.registry.as_mut().unwrap().resource_mut::<T::Target>()
                as *mut Option<&mut T::Target>)
                .cast::<Option<T>>()
                .read()
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

    fn get(state: &mut QueryState) -> Self {
        let c = unsafe {
            (state.ptr.add(state.translations[*state.idx]) as *const T)
                .as_ref()
                .unwrap()
        };
        *state.idx += 1;
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

    fn get(state: &mut QueryState) -> Self {
        let c = unsafe {
            (state.ptr.add(state.translations[*state.idx]) as *mut T)
                .as_mut()
                .unwrap()
        };
        *state.idx += 1;
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

    fn get(state: &mut QueryState) -> Self {
        unsafe {
            state
                .registry
                .as_mut()
                .unwrap()
                .resource()
                .expect("resource not found")
        }
    }

    fn refs(deps: &mut HashSet<TypeId>) {
        deps.insert(any::TypeId::of::<T>());
    }
}

impl<'a, T: Resource> Queryable<ResourceMarker> for &'a mut T {
    type Target = T;
    fn translations(
        translations: &mut Vec<usize>,
        storage_archetype: &Archetype,
        query_archetype: &Archetype,
    ) {
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

    fn get(state: &mut QueryState) -> Self {
        unsafe {
            state
                .registry
                .as_mut()
                .unwrap()
                .resource_mut()
                .expect("resource not found")
        }
    }

    fn refs(deps: &mut HashSet<TypeId>) {
        deps.insert(any::TypeId::of::<T>());
    }
    fn muts(deps: &mut HashSet<TypeId>) {
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

    fn get(state: &mut QueryState) -> Self {
        unsafe { ptr::read(state.entity) }
    }
}

pub struct Query<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> {
    registry: *mut Registry,
    commands: Commands,
    mapping: Vec<(Archetype, usize, usize, Vec<usize>, *mut u8, *const Entity)>,
    inner: usize,
    outer: usize,
    marker: PhantomData<(&'a Q, &'a QQ)>,
}

unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Send for Query<'a, QQ, Q> {}
unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Sync for Query<'a, QQ, Q> {}

pub trait QueryExt<QQ: ?Sized>: Queryable<QQ> {
    fn query(registry: &mut Registry) -> Query<QQ, Self>;
}

impl<'a, QQ: ?Sized, T: Queryable<QQ>> QueryExt<QQ> for T {
    fn query(registry: &mut Registry) -> Query<QQ, Self> {
        let mut query_archetype = Default::default();

        T::add(&mut query_archetype);

        let mut mapping = vec![];

        for (storage_archetype, storage) in &mut registry.inner.storage {
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
            registry,
            commands: Commands {
                tx: registry.inner.command_tx.clone(),
                handle: registry.handle(),
            },
            mapping,
            inner: 0,
            outer: 0,
            marker: PhantomData,
        }
    }
}

impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Iterator for Query<'a, QQ, Q> {
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

        Some(Q::get(&mut QueryState {
            registry: self.registry,
            commands: &mut self.commands,
            archetype,
            ptr,
            entity,
            translations,
            idx: &mut 0,
        }))
    }
}

pub struct ParallelQueryInner<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> {
    registry: *mut Registry,
    mapping: ParallelQueryPayload,
    inner: usize,
    commands: Commands,
    marker: PhantomData<(&'a Q, &'a QQ)>,
}

pub struct ParallelQueryOuter<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> {
    iter: Box<dyn Iterator<Item = Q> + Send + 'a>,
    marker: PhantomData<(&'a Q, &'a QQ)>,
}

unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> Send for ParallelQueryInner<'a, QQ, Q> {}
unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> Sync for ParallelQueryInner<'a, QQ, Q> {}

pub struct ParallelQuery<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> {
    iter: rayon::iter::IterBridge<ParallelQueryOuter<'a, QQ, Q>>,
    marker: PhantomData<(&'a Q, &'a QQ)>,
}

unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> Send for ParallelQueryOuter<'a, QQ, Q> {}
unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ> + ?Sized> Sync for ParallelQueryOuter<'a, QQ, Q> {}

pub struct ParallelQueryPayload {
    archetype: Archetype,
    len: usize,
    total_size: usize,
    translations: Vec<usize>,
    data: *mut u8,
    entities: *const Entity,
}

unsafe impl Send for ParallelQueryPayload {}
unsafe impl Sync for ParallelQueryPayload {}

band_proc_macro::make_query_tuples!(16);

unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Send for ParallelQuery<'a, QQ, Q> {}
unsafe impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Sync for ParallelQuery<'a, QQ, Q> {}

pub trait ParallelQueryExt<QQ: ?Sized>: Queryable<QQ> {
    fn par_query(registry: &mut Registry) -> ParallelQuery<QQ, Self>;
}

impl<'a, QQ: ?Sized, T: Queryable<QQ> + Send + QuerySync<QQ> + Sized> ParallelQueryExt<QQ> for T {
    fn par_query(registry: &mut Registry) -> ParallelQuery<QQ, Self> {
        let mut query_archetype = Default::default();

        T::add(&mut query_archetype);

        let mut mapping = vec![];

        for (storage_archetype, storage) in &mut registry.inner.storage {
            if storage.data.len() == 0 {
                continue;
            }

            if !storage_archetype.is_subset_of(query_archetype.clone()) {
                continue;
            }

            let mut translations = vec![];

            T::translations(&mut translations, &storage_archetype, &query_archetype);

            mapping.push(ParallelQueryPayload {
                archetype: storage_archetype.clone(),
                len: storage.data.len() / storage_archetype.total_size(),
                total_size: storage_archetype.total_size(),
                translations,
                data: storage.data.as_mut_ptr(),
                entities: storage.mapping.as_ptr(),
            });
        }

        let iter = ParallelQueryOuter {
            iter: Box::new(mapping.into_iter().flat_map(|mapping| ParallelQueryInner {
                registry,
                mapping,
                commands: Commands {
                    tx: registry.inner.command_tx.clone(),
                    handle: registry.handle(),
                },
                inner: 0,
                marker: PhantomData,
            })),
            marker: PhantomData,
        }
        .par_bridge();
        ParallelQuery {
            iter,
            marker: PhantomData,
        }
    }
}

impl<'a, QQ: ?Sized, T: Queryable<QQ> + Send + Sized> rayon::iter::ParallelIterator
    for ParallelQuery<'a, QQ, T>
{
    type Item = T;
    fn drive_unindexed<C: rayon::iter::plumbing::UnindexedConsumer<T>>(
        self,
        consumer: C,
    ) -> C::Result {
        self.iter.drive_unindexed(consumer)
    }
}

impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Iterator for ParallelQueryOuter<'a, QQ, Q> {
    type Item = Q;
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

impl<'a, QQ: ?Sized, Q: Queryable<QQ>> Iterator for ParallelQueryInner<'a, QQ, Q> {
    type Item = Q;
    fn next(&mut self) -> Option<Self::Item> {
        let ParallelQueryPayload {
            archetype,
            len,
            total_size,
            translations,
            data,
            entities,
        } = &self.mapping;

        if self.inner >= *len {
            None?
        }

        let ptr = unsafe { data.add(total_size * self.inner) };
        let entity = unsafe { entities.add(self.inner) };

        self.inner += 1;

        Some(Q::get(&mut QueryState {
            registry: self.registry,
            commands: &mut self.commands,
            archetype,
            ptr,
            entity,
            translations,
            idx: &mut 0,
        }))
    }
}
