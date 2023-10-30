use crate::{query::*, Registry};
use async_trait::async_trait;
use futures::Future;
use std::marker::PhantomData;
use std::{any::TypeId, collections::*};
use tokio::sync::{mpsc::*, oneshot};
#[async_trait]
pub trait System<T, QQ: ?Sized> {
    type Param: Queryable<QQ>;
    async fn execute(&self, payload: SystemPayload<Self::Param>);
    fn ref_deps(&self) -> HashSet<TypeId>;
    fn mut_deps(&self) -> HashSet<TypeId>;
}

pub struct SystemPayload<T> {
    registry: *mut Registry,
    param: *mut T,
}

unsafe impl<T> Send for SystemPayload<T> {}
unsafe impl<T> Sync for SystemPayload<T> {}

pub struct Seq;
pub struct Par;
pub struct Async;

pub struct WrapperSystem<'a, QQ: ?Sized, T: Queryable<QQ>, S> {
    sub_system: Box<dyn System<T, QQ, Param = T>>,
    marker: PhantomData<(&'a T, S)>,
}

unsafe impl<'a, QQ: ?Sized, T: Queryable<QQ> + Send, S> Send for WrapperSystem<'a, QQ, T, S> {}
unsafe impl<'a, QQ: ?Sized, T: Queryable<QQ> + Send, S> Sync for WrapperSystem<'a, QQ, T, S> {}

pub struct SubSystem<QQ: ?Sized, T: Queryable<QQ> + Send>(
    *mut Registry,
    *const dyn System<T, QQ, Param = T>,
);

impl<QQ: ?Sized, T: Queryable<QQ> + Send> Clone for SubSystem<QQ, T> {
    fn clone(&self) -> Self {
        Self(self.0, self.1)
    }
}

unsafe impl<QQ: ?Sized, T: Queryable<QQ> + Send> Send for SubSystem<QQ, T> {}
unsafe impl<QQ: ?Sized, T: Queryable<QQ> + Send> Sync for SubSystem<QQ, T> {}

#[async_trait]
impl<'a, QQ: ?Sized + 'static, A: Queryable<QQ> + Send + QuerySync<QQ> + 'static> System<(), ()>
    for WrapperSystem<'a, QQ, A, Seq>
{
    type Param = ();
    async fn execute(&self, SystemPayload { registry, .. }: SystemPayload<Self::Param>) {
        let sub_system = SubSystem::<QQ, A>(registry, &*self.sub_system as *const _);
        tokio::task::spawn_blocking(move || {
            let moved_sub_system = sub_system;
            A::query(unsafe { moved_sub_system.0.as_mut().unwrap() }).for_each(|mut param| {
                let SubSystem(registry, sub_system) = moved_sub_system.clone();
                futures::executor::block_on(unsafe { sub_system.as_ref() }.unwrap().execute(
                    SystemPayload {
                        registry,
                        param: &mut param as *mut _,
                    },
                ));
            });
        });
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
impl<'a, QQ: ?Sized + 'static, A: Queryable<QQ> + Send + QuerySync<QQ> + 'static> System<(), ()>
    for WrapperSystem<'a, QQ, A, Par>
{
    type Param = ();
    async fn execute(&self, SystemPayload { registry, .. }: SystemPayload<Self::Param>) {
        let sub_system = SubSystem::<QQ, A>(registry, &*self.sub_system as *const _);
        tokio::task::spawn_blocking(move || {
            let moved_sub_system = sub_system;
            use rayon::iter::ParallelIterator;
            A::par_query(unsafe { moved_sub_system.0.as_mut().unwrap() }).for_each(|mut param| {
                let SubSystem(registry, sub_system) = moved_sub_system.clone();
                futures::executor::block_on(unsafe { sub_system.as_ref() }.unwrap().execute(
                    SystemPayload {
                        registry,
                        param: &mut param as *mut _,
                    },
                ));
            });
        });
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
impl<'a, QQ: ?Sized + 'static, A: Queryable<QQ> + Send + QuerySync<QQ> + 'static> System<(), ()>
    for WrapperSystem<'a, QQ, A, Async>
{
    type Param = ();
    async fn execute(&self, SystemPayload { registry, .. }: SystemPayload<Self::Param>) {
        let sub_system = SubSystem::<QQ, A>(registry, &*self.sub_system as *const _);
        let params_and_futures = A::query(unsafe { registry.as_mut().unwrap() })
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
pub type Node = Box<dyn System<(), (), Param = ()>>;
#[derive(Default)]
pub struct Graph {
    nodes: Vec<Node>,
    dependencies: Vec<Vec<NodeId>>,
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
    pub fn new() -> Self {
        let (work_tx, work_rx) = unbounded_channel();

        Self {
            graph: Default::default(),
            work_tx,
            work_rx,
        }
    }
    pub fn add_seq<
        AA: 'static,
        A: Queryable<AA> + 'static + Send + QuerySync<AA>,
        T: System<A, AA, Param = A> + 'static,
    >(
        &mut self,
        system: T,
    ) {
        let sub_system = Box::new(system);
        let wrapper_system = Box::new(WrapperSystem::<AA, A, Seq> {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }
    pub fn add_par<
        AA: 'static,
        A: Queryable<AA> + 'static + Send + QuerySync<AA>,
        T: System<A, AA, Param = A> + 'static,
    >(
        &mut self,
        system: T,
    ) {
        let sub_system = Box::new(system);
        let wrapper_system = Box::new(WrapperSystem::<AA, A, Par> {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }
    pub fn add_async<
        AA: 'static,
        A: Queryable<AA> + 'static + Send + QuerySync<AA>,
        T: System<A, AA, Param = A> + 'static,
    >(
        &mut self,
        system: T,
    ) {
        let sub_system = Box::new(system);
        let wrapper_system = Box::new(WrapperSystem::<AA, A, Async> {
            sub_system,
            marker: PhantomData,
        });
        self.graph.add(wrapper_system);
    }
    pub async fn execute(&self, registry: &mut Registry) {
        fn work_executor(work_unit: WorkUnit) -> impl Future<Output = NodeId> + Send + 'static {
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(work_unit);
            async move {
                let WorkUnit(id, registry, system) = rx.await.expect("?");

                unsafe {
                    system
                        .as_ref()
                        .unwrap()
                        .execute(SystemPayload {
                            registry,
                            param: &mut (),
                        })
                        .await;
                }

                id
            }
        }

        let mut nodes_iter = self.graph.nodes.iter().enumerate().peekable();

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
                    .any(|id| self.graph.dependencies[nodes_iter.peek().unwrap().0].contains(&id))
                {
                    break;
                }

                let (id, node) = nodes_iter.next().unwrap();

                executing.push(id);

                let work_unit = WorkUnit(id, registry, (&**node) as *const _ as *mut _);

                let handle: tokio::task::JoinHandle<NodeId> =
                    tokio::spawn(work_executor(work_unit));

                futures.push(handle);
            }

            futures::future::join_all(futures).await;
        }
    }
}
