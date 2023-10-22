extern crate proc_macro;
use proc_macro::TokenStream;
use syn::{Lit, parse_macro_input};

#[proc_macro]
pub fn make_tuples(input: TokenStream) -> TokenStream {
    let Lit::Int(max_len_lit) = parse_macro_input!(input as Lit) else { panic!("?"); };
    let max_len = max_len_lit.base10_parse::<usize>().unwrap();
    let mut result = String::new();
    for len in 1..=max_len {
        result.push_str("impl<'a");
        for i in 0..len {
            let end = if i == len - 1 {
                ""
            } else {
                ", "
            };
            result.push_str(&format!("TMarker{}, ", i));
            result.push_str(&format!("T{}{}", i, end));
        }
        result.push_str("> Queryable<MultiMarker> for (");
        for i in 0..len {
            result.push_str(&format!("T{}", i));
        }
        result.push_str(")\n");

        result.push_str("type Target = Self;\n");

        result.push_str("fn translations(\n\
            translations: &mut Vec<usize>,\n\
            storage_archetype: &Archetype,\n\
            query_archetype: &Archetype,\n\
        ) {");

        for i in 0..len {
            result.push_str(&format!("T{}::translations(translations, storage_archetype, query_archetype);\n", i));
        }

        result.push_str("}\n");

        result.push_str("fn add(archetype: &mut Archetype) {\n");

        for i in 0..len {
            result.push_str(&format!("T{}::add(archetype);\n", i));
        }

        result.push_str("}\n");

        result.push_str("fn get(\n\
            archetype: &Archetype,\n\
            ptr: *mut u8,\n\
            entity: *const Entity, \n\
            translations: &[usize], \n\
            idx: &mut usize, \n\
        ) -> Self {\n(\n");

        for i in 0..len {
            result.push_str(&format!("T{}::get(archetype, ptr, entity, translations, idx);\n", i));
        }

        result.push_str("}\n}\n");

        result.push_str("fn refs(deps: &mut HashSet<TypeId>) {\n");

        for i in 0..len {
            result.push_str(&format!("T{}::::refs(deps);\n", i));
        }

        result.push_str("}\n");


        result.push_str("fn muts(deps: &mut HashSet<TypeId>) {\n");

        for i in 0..len {
            result.push_str(&format!("T{}::::muts(deps);\n", i));
        }

        result.push_str("}\n");
        result.push_str("}\n");
    }
    result.parse().unwrap()
}

#[proc_macro]
pub fn make_systems(input: TokenStream) -> TokenStream {
    let Lit::Int(max_len_lit) = parse_macro_input!(input as Lit) else { panic!("?"); };
    let max_len = max_len_lit.base10_parse::<usize>().unwrap();
    let mut result = String::new();
    result.push_str("impl<");
    for len in 1..=max_len {
        for i in 0..len {
            result.push_str(&format!("TT{}", i));
            result.push_str(&format!("T{}: Queryable<TT{}>, ", i, i));
        }
        result.push_str("Function: Fn(");
        for i in 0..len {
            let end = if i == len - 1 && len != 1 {
                ""
            } else {
                ", "
            };
            result.push_str(&format!("T{}{}", i, end));
        }
        result.push_str(")> for Function { \n");
        result.push_str("type Param = (");
        for i in 0..len {
            let end = if i == len - 1 {
                ""
            } else {
                ", "
            };
            result.push_str(&format!("T{}{}", i, end));
        }
        result.push_str(");\n");
        result.push_str("fn execute(&self, _: *mut Registry, (");
        for i in 0..len {
            let end = if i == len - 1 {
                ""
            } else {
                ", "
            };
            result.push_str(&format!("T{}{}", i, end));
        }
        result.push_str("): Self::Param) { self.call(");
        for i in 0..len {
            let end = if i == len - 1 && len != 1 {
                ""
            } else {
                ", "
            };
            result.push_str(&format!("T{}{}", i, end));
        }
        result.push_str(") }\n");

        result.push_str("\
            fn ref_deps(&self) -> HashSet<TypeId> { \n\
                unreachable!() \n\
            } \n\
            fn mut_deps(&self) -> HashSet<TypeId> {\n\
                unreachable!()\n\
            }\n\
        ");
        result.push_str("}\n");
    }
    result.parse().unwrap()
}