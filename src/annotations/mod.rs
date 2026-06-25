pub mod api;
pub mod builtins;
pub mod interp;
pub mod source_builders;
pub mod typed_builders;

use crate::analyzer::typed_ast::{TypedFile, TypedItem};
use crate::analyzer::AnalysisError;
use crate::annotations::api::{AnnotationArgs, AnnotationTarget, SourceAnnotationTarget};
use crate::annotations::interp::{ProcessorOutcome, Replacement};
use crate::diagnostics::timing::ProcessorRun;
use crate::parser::ast::{AnnotationDef, ImplBlock, Item, ProcessorDef, SourceFile, TypeExpr};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

type SourceProcessorFn =
    Box<dyn Fn(&SourceFile, SourceAnnotationTarget, AnnotationArgs) -> Vec<Item> + Send + Sync>;

type TypedProcessorFn =
    Box<dyn Fn(&TypedFile, AnnotationTarget, AnnotationArgs) -> Vec<TypedItem> + Send + Sync>;

pub struct ProcessorRegistry {
    source: HashMap<String, SourceProcessorFn>,
    typed: HashMap<String, TypedProcessorFn>,
}

impl ProcessorRegistry {
    pub fn new() -> Self {
        Self {
            source: HashMap::new(),
            typed: HashMap::new(),
        }
    }

    pub fn register_source<F>(&mut self, name: &str, f: F)
    where
        F: Fn(&SourceFile, SourceAnnotationTarget, AnnotationArgs) -> Vec<Item>
            + Send
            + Sync
            + 'static,
    {
        self.source.insert(name.to_string(), Box::new(f));
    }

    pub fn register<F>(&mut self, name: &str, f: F)
    where
        F: Fn(&TypedFile, AnnotationTarget, AnnotationArgs) -> Vec<TypedItem>
            + Send
            + Sync
            + 'static,
    {
        self.typed.insert(name.to_string(), Box::new(f));
    }

    fn get_source(&self, name: &str) -> Option<&SourceProcessorFn> {
        self.source.get(name)
    }

    fn get_typed(&self, name: &str) -> Option<&TypedProcessorFn> {
        self.typed.get(name)
    }
}

impl Default for ProcessorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Run source processors pre-analysis, appending generated Items to `source`.
pub fn run_source_processors(source: &mut SourceFile, registry: &ProcessorRegistry) {
    let mut new_items: Vec<Item> = Vec::new();

    let item_count = source.items.len();
    for idx in 0..item_count {
        let anns = match &source.items[idx] {
            Item::Function(f) => f.annotations.clone(),
            Item::Struct(s) => s.annotations.clone(),
            Item::Enum(e) => e.annotations.clone(),
            _ => continue,
        };

        for ann in &anns {
            let processor = match registry.get_source(&ann.name) {
                Some(p) => p,
                None => continue,
            };
            let target = match source_target_of(&source.items[idx]) {
                Some(t) => t,
                None => continue,
            };
            new_items.extend(processor(source, target, &ann.args));
        }
    }

    let existing_fns: HashSet<String> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Function(f) = i {
                Some(f.name.clone())
            } else {
                None
            }
        })
        .collect();

    for item in new_items {
        let is_dup = if let Item::Function(f) = &item {
            existing_fns.contains(&f.name)
        } else {
            false
        };
        if !is_dup {
            source.items.push(item);
        }
    }
}

/// Run typed processors post-analysis, appending generated TypedItems to `typed_file`.
pub fn run_processors(
    source: &SourceFile,
    typed_file: &mut TypedFile,
    registry: &ProcessorRegistry,
) {
    let mut new_items: Vec<TypedItem> = Vec::new();

    for item in &source.items {
        let anns = item_annotations(item);
        if anns.is_empty() {
            continue;
        }
        let name = match item_name(item) {
            Some(n) => n,
            None => continue,
        };
        for ann in anns {
            let processor = match registry.get_typed(&ann.name) {
                Some(p) => p,
                None => continue,
            };
            let target = match find_typed_target(typed_file, name, item) {
                Some(t) => t,
                None => continue,
            };
            let file_snapshot = typed_file.clone();
            new_items.extend(processor(&file_snapshot, target, &ann.args));
        }
    }

    let existing: HashSet<(String, String)> = typed_file
        .items
        .iter()
        .filter_map(|item| {
            if let TypedItem::ImplBlock(ib) = item {
                Some((ib.interface.clone(), ib.for_type.clone()))
            } else {
                None
            }
        })
        .collect();

    for item in new_items {
        let is_dup = if let TypedItem::ImplBlock(ib) = &item {
            existing.contains(&(ib.interface.clone(), ib.for_type.clone()))
        } else {
            false
        };
        if !is_dup {
            typed_file.items.push(item);
        }
    }
}

pub fn default_registry() -> ProcessorRegistry {
    let mut r = ProcessorRegistry::new();
    builtins::register_all(&mut r);
    r
}

/// Run user-defined Kiln processors over annotated items. Returns timing records.
pub fn run_user_processors(
    source: &mut SourceFile,
    errors: &mut Vec<AnalysisError>,
) -> Vec<ProcessorRun> {
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

    let ann_defs: Vec<AnnotationDef> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::AnnotationDef(a) = i {
                Some(a.clone())
            } else {
                None
            }
        })
        .collect();

    let mut new_items: Vec<Item> = vec![];
    let mut removed_indices: HashSet<usize> = HashSet::new();
    let mut proc_order: Vec<String> = vec![];
    let mut proc_stats: HashMap<String, (usize, std::time::Duration)> = HashMap::new();

    let item_count = source.items.len();
    for idx in 0..item_count {
        let annotations = match &source.items[idx] {
            Item::Function(f) => f.annotations.clone(),
            _ => continue,
        };

        for ann in annotations.iter().rev() {
            let proc = match procs.iter().find(|p| p.annotation_name == ann.name) {
                Some(p) => p.clone(),
                None => continue,
            };

            let fn_def = match &source.items[idx] {
                Item::Function(f) => f.clone(),
                _ => continue,
            };

            let resolved_args: Vec<(String, crate::parser::ast::Expr)> = {
                let mut merged = Vec::new();
                if let Some(adef) = ann_defs.iter().find(|a| a.name == ann.name) {
                    for field in &adef.fields {
                        if let Some(default_expr) = &field.default {
                            merged.push((field.name.clone(), default_expr.clone()));
                        }
                    }
                }
                for (k, v) in &ann.args {
                    if let Some(pos) = merged.iter().position(|(n, _)| n == k) {
                        merged[pos] = (k.clone(), v.clone());
                    } else {
                        merged.push((k.clone(), v.clone()));
                    }
                }
                merged
            };

            let t0 = Instant::now();
            let outcome = interp::Interpreter::run_processor(&fn_def, &proc, &resolved_args);
            let elapsed = t0.elapsed();

            fn apply_replacement(
                replacement: Replacement,
                idx: usize,
                items: &mut [Item],
                removed: &mut HashSet<usize>,
            ) {
                match replacement {
                    Replacement::Keep => {}
                    Replacement::Replace(rep) => items[idx] = *rep,
                    Replacement::Remove => {
                        removed.insert(idx);
                    }
                }
            }

            match outcome {
                ProcessorOutcome::Ok(replacement, extras) => {
                    apply_replacement(replacement, idx, &mut source.items, &mut removed_indices);
                    new_items.extend(extras);
                }
                ProcessorOutcome::Fail(msg) => {
                    errors.push(AnalysisError::ProcessorFail {
                        msg,
                        span: ann.span,
                    });
                }
                ProcessorOutcome::Warn(msgs, replacement, extras) => {
                    for msg in msgs {
                        errors.push(AnalysisError::ProcessorWarn {
                            msg,
                            span: ann.span,
                        });
                    }
                    apply_replacement(replacement, idx, &mut source.items, &mut removed_indices);
                    new_items.extend(extras);
                }
            }

            let entry = proc_stats.entry(ann.name.clone()).or_insert_with(|| {
                proc_order.push(ann.name.clone());
                (0, std::time::Duration::ZERO)
            });
            entry.0 += 1;
            entry.1 += elapsed;
        }
    }

    if !removed_indices.is_empty() {
        let mut idx_vec: Vec<usize> = removed_indices.into_iter().collect();
        idx_vec.sort_unstable_by(|a, b| b.cmp(a));
        for idx in idx_vec {
            source.items.remove(idx);
        }
    }

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

fn item_name(item: &Item) -> Option<&str> {
    match item {
        Item::Function(f) => Some(&f.name),
        Item::Struct(s) => Some(&s.name),
        Item::Enum(e) => Some(&e.name),
        _ => None,
    }
}

fn source_target_of(item: &Item) -> Option<SourceAnnotationTarget<'_>> {
    match item {
        Item::Function(f) => Some(SourceAnnotationTarget::Function(f)),
        Item::Struct(s) => Some(SourceAnnotationTarget::Struct(s)),
        Item::Enum(e) => Some(SourceAnnotationTarget::Enum(e)),
        _ => None,
    }
}

fn find_typed_target<'a>(
    typed_file: &'a TypedFile,
    name: &str,
    untyped: &Item,
) -> Option<AnnotationTarget<'a>> {
    for item in &typed_file.items {
        match (untyped, item) {
            (Item::Struct(_), TypedItem::Struct(s)) if s.name == name => {
                return Some(AnnotationTarget::Struct(s));
            }
            (Item::Enum(_), TypedItem::Enum(e)) if e.name == name => {
                return Some(AnnotationTarget::Enum(e));
            }
            (Item::Function(_), TypedItem::Function(f)) if f.name == name => {
                return Some(AnnotationTarget::Function(f));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::typed_ast::TypedFile;
    use crate::diagnostics::Span;

    fn empty_typed_file() -> TypedFile {
        TypedFile {
            items: vec![],
            span: Span::new(0, 0),
        }
    }

    fn empty_source() -> SourceFile {
        SourceFile {
            items: vec![],
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn registry_registers_and_retrieves_typed_processor() {
        let mut registry = ProcessorRegistry::new();
        registry.register("Log", |_file, _target, _args| vec![]);
        assert!(registry.get_typed("Log").is_some());
        assert!(registry.get_typed("Unknown").is_none());
    }

    #[test]
    fn registry_registers_and_retrieves_source_processor() {
        let mut registry = ProcessorRegistry::new();
        registry.register_source("gen", |_file, _target, _args| vec![]);
        assert!(registry.get_source("gen").is_some());
        assert!(registry.get_source("unknown").is_none());
    }

    #[test]
    fn run_source_processors_on_empty_source_does_nothing() {
        let registry = ProcessorRegistry::new();
        let mut source = empty_source();
        run_source_processors(&mut source, &registry);
        assert_eq!(source.items.len(), 0);
    }

    #[test]
    fn run_processors_on_empty_source_does_nothing() {
        let registry = ProcessorRegistry::new();
        let source = empty_source();
        let mut typed_file = empty_typed_file();
        run_processors(&source, &mut typed_file, &registry);
        assert_eq!(typed_file.items.len(), 0);
    }
}
