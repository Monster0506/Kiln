pub mod api;
pub mod builtins;
pub mod interp;

use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::parser::ast::{ImplBlock, Item, ProcessorDef, SourceFile, TypeExpr};
use std::collections::{HashMap, HashSet};

type ProcessorFn = Box<dyn Fn(AnnotationTarget, AnnotationArgs) -> Vec<Item> + Send + Sync>;

pub struct ProcessorRegistry {
    processors: HashMap<String, ProcessorFn>,
}

impl ProcessorRegistry {
    pub fn new() -> Self {
        Self {
            processors: HashMap::new(),
        }
    }

    pub fn register<F>(&mut self, name: &str, f: F)
    where
        F: Fn(AnnotationTarget, AnnotationArgs) -> Vec<Item> + Send + Sync + 'static,
    {
        self.processors.insert(name.to_string(), Box::new(f));
    }

    pub fn get(&self, name: &str) -> Option<&ProcessorFn> {
        self.processors.get(name)
    }
}

impl Default for ProcessorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Run all registered processors over every annotated item in `source`.
/// New items produced by processors are appended to `source.items`.
/// ImplBlocks are deduplicated: if an explicit impl for (interface, type) already
/// exists in the source, the derived one is dropped.
pub fn run_processors(source: &mut SourceFile, registry: &ProcessorRegistry) {
    let mut new_items: Vec<Item> = Vec::new();

    for item in &source.items {
        let annotations = item_annotations(item);
        for ann in annotations {
            if let Some(processor) = registry.get(&ann.name) {
                if let Some(target) = make_target(item) {
                    let generated = processor(target, &ann.args);
                    new_items.extend(generated);
                }
            }
        }
    }

    // Build a set of (interface_name, for_type_name) for all existing impl blocks,
    // then drop any generated impl that would duplicate an existing one.
    let existing: HashSet<(String, String)> = source
        .items
        .iter()
        .filter_map(|item| {
            if let Item::ImplBlock(ib) = item {
                impl_key(ib)
            } else {
                None
            }
        })
        .collect();

    for item in new_items {
        let is_dup = if let Item::ImplBlock(ib) = &item {
            impl_key(ib).is_some_and(|k| existing.contains(&k))
        } else {
            false
        };
        if !is_dup {
            source.items.push(item);
        }
    }
}

/// Run user-defined `processor` bodies (written in Kiln) over each annotated item.
/// Processors are looked up by annotation name from the `ProcessorDef` items in `source`.
/// Results (replacements and new items) are applied the same way as `run_processors`.
pub fn run_user_processors(source: &mut SourceFile, _registry: &ProcessorRegistry) {
    // Collect processor defs by annotation name.
    let procs: Vec<ProcessorDef> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::ProcessorDef(p) = i {
                Some(p.clone())
            } else {
                None
            }
        })
        .collect();

    if procs.is_empty() {
        return;
    }

    let mut replacements: Vec<(usize, Item)> = vec![];
    let mut new_items: Vec<Item> = vec![];

    for (idx, item) in source.items.iter().enumerate() {
        let (annotations, fn_def) = match item {
            Item::Function(f) => (&f.annotations, f),
            _ => continue,
        };

        for ann in annotations {
            let proc = match procs.iter().find(|p| p.annotation_name == ann.name) {
                Some(p) => p,
                None => continue,
            };

            if let Some((replacement, extras)) =
                interp::Interpreter::run_processor(fn_def, proc, &ann.args)
            {
                if let Some(rep) = replacement {
                    replacements.push((idx, rep));
                }
                new_items.extend(extras);
            }
        }
    }

    // Apply replacements (last-write wins if multiple processors touch the same item).
    for (idx, item) in replacements {
        source.items[idx] = item;
    }

    // Deduplicate new impl blocks, then extend.
    let existing: HashSet<(String, String)> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::ImplBlock(ib) = i {
                impl_key(ib)
            } else {
                None
            }
        })
        .collect();

    for item in new_items {
        let is_dup = if let Item::ImplBlock(ib) = &item {
            impl_key(ib).is_some_and(|k| existing.contains(&k))
        } else {
            false
        };
        if !is_dup {
            source.items.push(item);
        }
    }
}

/// Build the default registry with all built-in processors registered.
pub fn default_registry() -> ProcessorRegistry {
    let mut r = ProcessorRegistry::new();
    builtins::register_all(&mut r);
    r
}

fn impl_key(ib: &ImplBlock) -> Option<(String, String)> {
    let iface = match &ib.interface {
        TypeExpr::Named { name, .. } => name.clone(),
        _ => return None,
    };
    let for_ty = match &ib.for_type {
        TypeExpr::Named { name, .. } => name.clone(),
        _ => return None,
    };
    Some((iface, for_ty))
}

fn item_annotations(item: &Item) -> &[crate::parser::ast::AnnotationUse] {
    match item {
        Item::Function(f) => &f.annotations,
        Item::Struct(s) => &s.annotations,
        Item::Enum(e) => &e.annotations,
        _ => &[],
    }
}

fn make_target(item: &Item) -> Option<AnnotationTarget<'_>> {
    match item {
        Item::Function(f) => Some(AnnotationTarget::Function(f)),
        Item::Struct(s) => Some(AnnotationTarget::Struct(s)),
        Item::Enum(e) => Some(AnnotationTarget::Enum(e)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::*;

    fn s() -> Span {
        Span::new(0, 0)
    }

    #[test]
    fn registry_dispatches_to_registered_processor() {
        let mut registry = ProcessorRegistry::new();
        registry.register("Log", |_target, _args| vec![]);
        let fn_def = FnDef {
            annotations: vec![AnnotationUse {
                name: "Log".into(),
                args: vec![],
                span: s(),
            }],
            name: "foo".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: TypeExpr::Named {
                name: "void".into(),
                generics: vec![],
                bindings: vec![],
                span: s(),
            },
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        let mut source = SourceFile {
            items: vec![Item::Function(fn_def)],
            span: s(),
        };
        run_processors(&mut source, &registry);
        assert_eq!(source.items.len(), 1); // no new items from no-op processor
    }

    #[test]
    fn unknown_annotation_is_ignored() {
        let registry = ProcessorRegistry::new();
        let fn_def = FnDef {
            annotations: vec![AnnotationUse {
                name: "UnknownAnnotation".into(),
                args: vec![],
                span: s(),
            }],
            name: "bar".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: TypeExpr::Named {
                name: "void".into(),
                generics: vec![],
                bindings: vec![],
                span: s(),
            },
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        let mut source = SourceFile {
            items: vec![Item::Function(fn_def)],
            span: s(),
        };
        run_processors(&mut source, &registry);
        assert_eq!(source.items.len(), 1);
    }
}
