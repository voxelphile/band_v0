use crate::{archetype::*, entity::*, Component};
use std::collections::HashMap;
pub struct Storage {
    pub(crate) archetype: Archetype,
    pub(crate) data: Vec<u8>,
    pub(crate) mapping: Vec<Entity>,
    pub(crate) secondary_mapping: HashMap<Entity, usize>,
    pub(crate) table: Vec<Vec<Box<dyn Component>>>,
}

impl Storage {
    pub fn new(archetype: Archetype) -> Self {
        Self {
            archetype,
            data: vec![],
            mapping: vec![],
            secondary_mapping: HashMap::new(),
            table: vec![],
        }
    }
    pub fn push(&mut self, entity: Entity, data: Vec<u8>, repr: Vec<Box<dyn Component>>) {
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

    pub fn remove(&mut self, entity: Entity) -> (Vec<u8>, Vec<Box<dyn Component>>) {
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
