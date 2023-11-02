use crate::{query::*, Registry};
use async_trait::async_trait;
use futures::Future;
use pathfinding::prelude::connected_components;
use std::marker::PhantomData;
use std::sync::Arc;
use std::{any::TypeId, collections::*};
use tokio::sync::{mpsc::*, oneshot};
#[async_trait]
pub trait System<T: Send + Sync, QQ: ?Sized + Send + Sync>: Send + Sync {
    type Param: Queryable<QQ> + Send + Sync;
    async fn execute(&self, payload: SystemPayload<Self::Param>);
    fn ref_deps(&self) -> HashSet<TypeId>;
    fn mut_deps(&self) -> HashSet<TypeId>;
}

pub struct SystemPayload<T> {
    registry: RegistryPtr,
    param: *mut T,
}

unsafe impl<T> Send for SystemPayload<T> {}
unsafe impl<T> Sync for SystemPayload<T> {}

pub struct Seq;
pub struct Par;
pub struct Async;

pub struct WrapperSystem<'a, QQ: ?Sized, T: Queryable<QQ>, S> {
    sub_system: Arc<dyn System<T, QQ, Param = T>>,
    marker: PhantomData<(&'a T, S)>,
}

unsafe impl<'a, QQ: ?Sized, T: Queryable<QQ> + Send + Sync, S> Send
    for WrapperSystem<'a, QQ, T, S>
{
}
unsafe impl<'a, QQ: ?Sized, T: Queryable<QQ> + Send + Sync, S> Sync
    for WrapperSystem<'a, QQ, T, S>
{
}

#[derive(Copy)]
pub struct SubSystem<QQ: ?Sized, T: Queryable<QQ> + Send + Sync>(
    RegistryPtr,
    *const dyn System<T, QQ, Param = T>,
);

impl<QQ: ?Sized, T: Queryable<QQ> + Send + Sync> Clone for SubSystem<QQ, T> {
    fn clone(&self) -> Self {
        Self(self.0, self.1)
    }
}

unsafe impl<QQ: ?Sized, T: Queryable<QQ> + Send + Sync> Send for SubSystem<QQ, T> {}
unsafe impl<QQ: ?Sized, T: Queryable<QQ> + Send + Sync> Sync for SubSystem<QQ, T> {}

#[async_trait]
impl<
        'a,
        QQ: Send + Sync + ?Sized + 'static,
        A: Queryable<QQ> + Send + Sync + QuerySync<QQ> + 'static,
    > System<(), ()> for WrapperSystem<'a, QQ, A, Seq>
{
    type Param = ();
    async fn execute(&self, SystemPayload { registry, .. }: SystemPayload<Self::Param>) {
        let sub_system = SubSystem::<QQ, A>(registry, &*self.sub_system as *const _);
        tokio::task::spawn_blocking(move || {
            let moved_sub_system = sub_system;
            let RegistryPtr(registry_ptr) = moved_sub_system.0;
            A::query(unsafe { registry_ptr.as_mut().unwrap() }).for_each(|mut param| {
                let SubSystem(registry, sub_system) = moved_sub_system.clone();
                futures::executor::block_on(unsafe { sub_system.as_ref() }.unwrap().execute(
                    SystemPayload {
                        registry,
                        param: &mut param as *mut _,
                    },
                ));
            });
        })
        .await;
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
#[async_trait]
impl<
        'a,
        QQ: Send + Sync + ?Sized + 'static,
        A: Queryable<QQ> + Send + Sync + QuerySync<QQ> + 'static,
    > System<(), ()> for WrapperSystem<'a, QQ, A, Par>
{
    type Param = ();
    async fn execute(&self, SystemPayload { registry, .. }: SystemPayload<Self::Param>) {
        let sub_system = SubSystem::<QQ, A>(registry, &*self.sub_system as *const _);
        tokio::task::spawn_blocking(move || {
            let moved_sub_system = sub_system;
            use rayon::iter::ParallelIterator;
            A::par_query(unsafe { moved_sub_system.0 .0.as_mut().unwrap() }).for_each(
                |mut param| {
                    let SubSystem(registry, sub_system) = moved_sub_system.clone();
                    futures::executor::block_on(unsafe { sub_system.as_ref() }.unwrap().execute(
                        SystemPayload {
                            registry,
                            param: &mut param as *mut _,
                        },
                    ));
                },
            );
        })
        .await;
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
#[async_trait]
impl<
        'a,
        QQ: Send + Sync + ?Sized + 'static,
        A: Queryable<QQ> + Send + Sync + QuerySync<QQ> + 'static,
    > System<(), ()> for WrapperSystem<'a, QQ, A, Async>
{
    type Param = ();
    async fn execute(&self, SystemPayload { registry, .. }: SystemPayload<Self::Param>) {
        let sub_system = SubSystem::<QQ, A>(registry, &*self.sub_system as *const _);
        let params_and_futures = A::query(unsafe { registry.0.as_mut().unwrap() })
            .map(|param| {
                let SubSystem(registry, sub_system) = sub_system.clone();
                let mut param = Box::new(param);
                let future = unsafe {
                    sub_system.as_ref().unwrap().execute(SystemPayload {
                        registry,
                        param: param.as_mut() as *mut _,
                    })
                };
                (param, future)
            })
            .collect::<Vec<_>>();
        let (_params, futures) = params_and_futures
            .into_iter()
            .unzip::<_, _, Vec<_>, Vec<_>>();
        futures::future::join_all(futures).await;
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
pub type Node = std::sync::Arc<dyn System<(), (), Param = ()>>;
#[derive(Default, Clone)]
pub struct Graph {
    nodes: Vec<Node>,
    dependencies: Vec<Vec<NodeId>>,
    connected_sets: Vec<HashSet<NodeId>>,
}

impl Graph {
    fn add(&mut self, node: Node) {
        let my_id = self.nodes.len();

        self.nodes.push(node);

        let me = &self.nodes[my_id];

        let my_refs = me.ref_deps();
        let my_muts = me.mut_deps();

        let mut dependencies = Vec::new();

        'a: for (their_id, them) in self.nodes[..my_id].iter().enumerate().rev() {
            let their_refs = them.ref_deps();
            let their_muts = them.mut_deps();

            for my_ref in &my_refs {
                if their_muts.contains(my_ref) {
                    dependencies.push(their_id);
                    continue 'a;
                }
            }

            for my_mut in &my_muts {
                if their_refs.contains(my_mut) || their_muts.contains(my_mut) {
                    dependencies.push(their_id);
                    continue 'a;
                }
            }
        }

        self.dependencies.push(dependencies);

        self.connected_sets();
    }

    fn connected_sets(&mut self) {
        let mut starts = vec![];
        for node in (0..self.nodes.len()).rev() {
            if self.dependencies[node].iter().any(|id| starts.contains(id)) {
                break;
            }
            starts.push(node);
        }
        self.connected_sets =
            connected_components(&starts, |id| self.dependencies[*id].iter().copied());
    }
}

#[derive(Default)]
pub struct Scheduler {
    graph: Graph,
}

pub struct WorkUnit(NodeId, RegistryPtr, SystemPtr);
unsafe impl Send for WorkUnit {}
unsafe impl Sync for WorkUnit {}

#[derive(Clone, Copy)]
pub struct RegistryPtr(*mut Registry);
unsafe impl Send for RegistryPtr {}
unsafe impl Sync for RegistryPtr {}
#[derive(Clone, Copy)]
pub struct SystemPtr(*const (dyn System<(), (), Param = ()> + Send + Sync));
unsafe impl Send for SystemPtr {}
unsafe impl Sync for SystemPtr {}

impl Scheduler {
    pub fn add_seq<
        AA: Send + Sync + 'static,
        A: Queryable<AA> + 'static + Send + Sync + QuerySync<AA>,
        T: System<A, AA, Param = A> + 'static,
    >(
        &mut self,
        system: T,
    ) {
        let sub_system = Arc::new(system);
        let wrapper_system = std::sync::Arc::new(WrapperSystem::<AA, A, Seq> {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }
    pub fn add_par<
        AA: Send + Sync + 'static,
        A: Queryable<AA> + 'static + Send + Sync + QuerySync<AA>,
        T: System<A, AA, Param = A> + 'static,
    >(
        &mut self,
        system: T,
    ) {
        let sub_system = Arc::new(system);
        let wrapper_system = std::sync::Arc::new(WrapperSystem::<AA, A, Par> {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }
    pub fn add_async<
        AA: Send + Sync + 'static,
        A: Queryable<AA> + 'static + Send + Sync + QuerySync<AA>,
        T: System<A, AA, Param = A> + 'static,
    >(
        &mut self,
        system: T,
    ) {
        let sub_system = Arc::new(system);
        let wrapper_system = std::sync::Arc::new(WrapperSystem::<AA, A, Async> {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }

    pub async fn execute(&self, registry: &mut Registry) {
        let registry_ptr = RegistryPtr(registry);

        async fn work_executor(rx: oneshot::Receiver<WorkUnit>) -> NodeId {
            let WorkUnit(id, registry, system) = rx.await.expect("?");
            let payload = SystemPayload {
                registry,
                param: &mut (),
            };
            unsafe {
                let system_ref = system.0.as_ref().unwrap();

                system_ref.execute(payload).await;
            }

            id
        }

        let _handle = registry.handle();

        let mut masters = vec![];

        for set in &self.graph.connected_sets {
            let dependencies = self.graph.dependencies.clone();

            let nodes = self
                .graph
                .nodes
                .clone()
                .into_iter()
                .enumerate()
                .filter(|(id, _)| set.contains(id))
                .collect::<Vec<_>>();

            masters.push(tokio::spawn(async move {
                let registry = registry_ptr;
                let mut nodes_iter = nodes.into_iter().peekable();

                'a: loop {
                    let mut futures = Vec::new();
                    let mut executing = Vec::<NodeId>::new();

                    loop {
                        if nodes_iter.peek().is_none() {
                            futures::future::join_all(futures).await;
                            break 'a;
                        }

                        if executing
                            .iter()
                            .any(|id| dependencies[nodes_iter.peek().unwrap().0].contains(id))
                        {
                            break;
                        }

                        let (id, node) = nodes_iter.next().unwrap();

                        executing.push(id);

                        let work_unit = WorkUnit(
                            id,
                            registry,
                            SystemPtr(std::sync::Arc::as_ptr(&node) as *const _),
                        );

                        let (tx, rx) = oneshot::channel();
                        let _ = tx.send(work_unit);

                        let handle: tokio::task::JoinHandle<NodeId> =
                            tokio::spawn(work_executor(rx));

                        futures.push(handle);
                    }

                    futures::future::join_all(futures).await;
                }
            }));
        }

        futures::future::join_all(masters).await;
    }
}
