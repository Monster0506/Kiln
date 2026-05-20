pub mod api;
pub mod builtins;
pub mod interp;

use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::diagnostics::timing::ProcessorRun;
use crate::parser::ast::{ImplBlock, Item, ProcessorDef, SourceFile, TypeExpr};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

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
/// Returns per-processor timing records for use with `BuildStats`.
pub fn run_user_processors(
    source: &mut SourceFile,
    _registry: &ProcessorRegistry,
) -> Vec<ProcessorRun> {
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
        return vec![];
    }

    let mut new_items: Vec<Item> = vec![];
    // Track (item_count, total_duration) per processor name, in insertion order.
    let mut proc_order: Vec<String> = vec![];
    let mut proc_stats: HashMap<String, (usize, std::time::Duration)> = HashMap::new();

    // Process each item's annotations in declaration order, chaining outputs:
    // each processor in a stack sees the output of the previous one.
    let item_count = source.items.len();
    for idx in 0..item_count {
        // Clone the annotation list from the original function once, so the
        // loop order is stable even as source.items[idx] is mutated.
        let annotations = match &source.items[idx] {
            Item::Function(f) => f.annotations.clone(),
            _ => continue,
        };

        for ann in &annotations {
            let proc = match procs.iter().find(|p| p.annotation_name == ann.name) {
                Some(p) => p.clone(),
                None => continue,
            };

            // Re-read the current item so each processor sees the previous output.
            let fn_def = match &source.items[idx] {
                Item::Function(f) => f.clone(),
                _ => continue,
            };

            let t0 = Instant::now();
            let result = interp::Interpreter::run_processor(&fn_def, &proc, &ann.args);
            let elapsed = t0.elapsed();

            if let Some((replacement, extras)) = result {
                if let Some(rep) = replacement {
                    source.items[idx] = rep;
                }
                new_items.extend(extras);
            }

            let entry = proc_stats.entry(ann.name.clone()).or_insert_with(|| {
                proc_order.push(ann.name.clone());
                (0, std::time::Duration::ZERO)
            });
            entry.0 += 1;
            entry.1 += elapsed;
        }
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

    proc_order
        .into_iter()
        .map(|name| {
            let (item_count, duration) = proc_stats[&name];
            ProcessorRun {
                name,
                item_count,
                duration,
            }
        })
        .collect()
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
