use std::any::TypeId;
#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub enum ArchetypeState {
    Component,
    Optional,
    Without,
    Resource,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub struct ArchetypeInfo {
    pub(crate) ty: TypeId,
    pub(crate) size: usize,
    pub(crate) name: &'static str,
    pub(crate) state: ArchetypeState,
}

#[derive(Default, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Archetype {
    pub(crate) len: usize,
    pub(crate) info: [Option<ArchetypeInfo>; 32],
}

impl Archetype {
    pub fn translation(&self, ty: TypeId) -> Option<usize> {
        let Some((idx, _)) = self
            .info
            .iter()
            .take(self.len)
            .cloned()
            .map(Option::unwrap)
            .enumerate()
            .map(|(i, x)| (i, x.ty))
            .find(|(_, x)| *x == ty)
        else {
            return None;
        };

        Some(self.size_up_to(idx))
    }
    pub fn is_subset_of(&self, superset: Archetype) -> bool {
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
    pub fn total_size(&self) -> usize {
        self.size_up_to(self.len)
    }

    pub fn size_up_to(&self, idx: usize) -> usize {
        self.info
            .iter()
            .cloned()
            .take(idx)
            .map(Option::unwrap)
            .fold(0, |sum, info| sum + info.size)
    }
}
