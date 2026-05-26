pub mod cfg;
pub mod check;
pub mod collect;
pub mod conformance;
pub mod constrain;
pub mod constraints;
pub mod dce;
pub mod env;
pub mod error;
pub mod escape;
pub mod exhaustive;
pub mod fold;
pub mod infer;
pub mod infer_bounds;
pub mod liveness;
pub mod op_hierarchy;
pub mod opt_notes;
pub mod pretty;
pub mod prop;
pub mod purity;
pub mod resolve;
pub mod returns;
pub mod solve;
pub mod ty;
pub mod typed_ast;
pub mod unroll;

pub use error::AnalysisError;
pub use ty::{Ty, TypeId, TypeRegistry};
pub use typed_ast::TypedFile;

use crate::analyzer::env::{Env, FnOverload, GenericBound, Symbol};
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::MethodEntry;
use crate::analyzer::typed_ast::{
    TypedEnumDef, TypedEnumVariant, TypedField, TypedFnDef, TypedGlobalVar, TypedHookDef,
    TypedImplBlock, TypedInterfaceDef, TypedInterfaceMethod, TypedItem, TypedParam, TypedStructDef,
};
use crate::diagnostics::Span;
use crate::parser::ast::{Expr, HookName, ImplKind, Item, SourceFile, StringSegment, TypeExpr};

fn register_builtins(_env: &mut Env, _registry: &mut ty::TypeRegistry) {
    // All builtin names are declared in prelude.kn.
    // None and Some are registered via @builtin def declarations in the prelude;
    // their symbols are resolved in pass 1b alongside all other function signatures.
}

/// Alpha-normalized type equality for signature comparison.
/// All GenericParam variants are treated as equal to each other when compared positionally.
fn ty_sig_eq(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::GenericParam(_), Ty::GenericParam(_)) => true,
        (Ty::Named(_, na, aa), Ty::Named(_, nb, ab)) => {
            na == nb && aa.len() == ab.len() && aa.iter().zip(ab).all(|(x, y)| ty_sig_eq(x, y))
        }
        (Ty::Callable(pa, ra), Ty::Callable(pb, rb)) => {
            pa.len() == pb.len()
                && pa.iter().zip(pb).all(|(x, y)| ty_sig_eq(x, y))
                && ty_sig_eq(ra, rb)
        }
        _ => a == b,
    }
}

/// Returns true if two parameter lists have the same arity and pairwise-equal types
/// under alpha-normalization of generic params.
fn params_are_duplicate(a: &[(String, Ty)], b: &[(String, Ty)]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|((_, ta), (_, tb))| ty_sig_eq(ta, tb))
}

/// Replace every `Ty::GenericParam` whose name appears in `params` with `Ty::Unknown`.
/// Used to produce an instantiated-but-unconstrained type for 0-arg value declarations.
fn subst_generic_params_unknown(ty: &Ty, params: &[String]) -> Ty {
    match ty {
        Ty::GenericParam(p) if params.contains(p) => Ty::Unknown,
        Ty::Named(id, name, args) => {
            let new_args = args
                .iter()
                .map(|a| subst_generic_params_unknown(a, params))
                .collect();
            Ty::Named(id.clone(), name.clone(), new_args)
        }
        Ty::Callable(param_tys, ret) => {
            let new_params = param_tys
                .iter()
                .map(|p| subst_generic_params_unknown(p, params))
                .collect();
            Ty::Callable(
                new_params,
                Box::new(subst_generic_params_unknown(ret, params)),
            )
        }
        other => other.clone(),
    }
}

/// Extract the interface name from a bound TypeExpr (e.g. `Iterator[Item=int]` -> `"Iterator"`).
fn bound_iface_name(tex: &TypeExpr, env: &Env) -> Option<String> {
    if let TypeExpr::Named { name, .. } = tex {
        // If the name is a type alias for an interface, resolve through it.
        if let Some(crate::analyzer::env::Symbol::TypeAlias(crate::analyzer::ty::Ty::Interface(
            _,
            real,
        ))) = env.lookup(name)
        {
            return Some(real.clone());
        }
        Some(name.clone())
    } else {
        None
    }
}

/// Resolve assoc bindings from a bound TypeExpr. For `Iterator[Item=int]` returns
/// `[("Item", Ty::Int)]`; for `Iterator[Item=Display]` returns `[("Item", Ty::Interface(...))]`.
fn bound_assoc_bindings(
    tex: &TypeExpr,
    env: &Env,
    errors: &mut Vec<AnalysisError>,
) -> Vec<(String, Ty)> {
    if let TypeExpr::Named { bindings, .. } = tex {
        bindings
            .iter()
            .map(|(name, ty_expr)| (name.clone(), resolve_type_expr(ty_expr, env, errors)))
            .collect()
    } else {
        vec![]
    }
}

/// Build GenericBounds from a slice of GenericParam, resolving assoc bindings.
fn build_generic_bounds(
    generic_params: &[crate::parser::ast::GenericParam],
    env: &Env,
    errors: &mut Vec<AnalysisError>,
) -> Vec<GenericBound> {
    let mut bounds = Vec::new();
    for g in generic_params {
        for b in &g.bounds {
            if let Some(iface) = bound_iface_name(b, env) {
                let assoc_bindings = bound_assoc_bindings(b, env, errors);
                bounds.push(GenericBound {
                    param: g.name.clone(),
                    iface,
                    assoc_bindings,
                    is_explicit: true,
                    decl_span: Some(g.span),
                    source_span: None,
                    source_desc: String::new(),
                });
            }
        }
    }
    bounds
}

/// Register projection pins from generic params into the env's innermost scope.
/// Call this after push_scope() and after generic params are defined as Symbol::Type.
fn register_projection_pins(
    generic_params: &[crate::parser::ast::GenericParam],
    env: &mut Env,
    errors: &mut Vec<AnalysisError>,
) {
    for gp in generic_params {
        for b in &gp.bounds {
            if let TypeExpr::Named {
                name: iface_name,
                bindings,
                ..
            } = b
            {
                // Track that this param is bounded by this interface (for method dispatch).
                env.register_param_iface(&gp.name, iface_name);
                for (assoc_name, binding_ty_expr) in bindings {
                    let binding_ty = resolve_type_expr(binding_ty_expr, env, errors);
                    // Only pin if it's a concrete type (not an interface bound like Item=Display).
                    if !matches!(binding_ty, Ty::Interface(_, _)) {
                        env.pin_projection(&gp.name, assoc_name, binding_ty);
                    }
                }
            }
        }
    }
}

/// Analyze `source`, producing a `TypedFile` or a list of errors.
///
/// The stdlib prelude is always prepended before user items.
pub fn analyze(source: &SourceFile) -> Result<TypedFile, Vec<AnalysisError>> {
    analyze_with_base(source, &std::path::PathBuf::from("."))
}

/// Like `analyze`, but resolves `import` statements relative to `base_dir`.
pub fn analyze_with_base(
    source: &SourceFile,
    base_dir: &std::path::Path,
) -> Result<TypedFile, Vec<AnalysisError>> {
    let prelude_src = crate::stdlib::parse_prelude();
    let ast_stdlib = crate::stdlib::parse_ast_stdlib();
    let stdlib_vfs = crate::stdlib::stdlib_virtual_fs();

    let mut import_errors: Vec<AnalysisError> = Vec::new();

    // Resolve prelude imports using the embedded stdlib virtual filesystem.
    let mut prelude_items: Vec<Item> = Vec::new();
    resolve_imports_into(
        &prelude_src,
        base_dir,
        &mut prelude_items,
        &mut import_errors,
        &stdlib_vfs,
    );

    // Resolve user imports from disk (vfs is also passed for any stdlib re-imports).
    let mut user_items: Vec<Item> = Vec::new();
    resolve_imports_into(
        source,
        base_dir,
        &mut user_items,
        &mut import_errors,
        &stdlib_vfs,
    );

    let combined_items: Vec<_> = prelude_items
        .into_iter()
        .chain(ast_stdlib.items)
        .chain(user_items)
        .collect();
    let combined = SourceFile {
        items: combined_items,
        span: source.span,
    };

    match analyze_inner(&combined) {
        Ok((typed, _registry)) if import_errors.is_empty() => Ok(typed),
        Ok(_) => Err(import_errors),
        Err(mut errs) => {
            import_errors.append(&mut errs);
            Err(import_errors)
        }
    }
}

/// Like `analyze_with_base`, but also returns the `TypeRegistry` built during analysis.
/// Used by codegen paths that need the registry for monomorphization.
pub fn analyze_with_base_and_registry(
    source: &SourceFile,
    base_dir: &std::path::Path,
) -> Result<(TypedFile, ty::TypeRegistry), Vec<AnalysisError>> {
    let prelude_src = crate::stdlib::parse_prelude();
    let ast_stdlib = crate::stdlib::parse_ast_stdlib();
    let stdlib_vfs = crate::stdlib::stdlib_virtual_fs();

    let mut import_errors: Vec<AnalysisError> = Vec::new();

    let mut prelude_items: Vec<Item> = Vec::new();
    resolve_imports_into(
        &prelude_src,
        base_dir,
        &mut prelude_items,
        &mut import_errors,
        &stdlib_vfs,
    );

    let mut user_items: Vec<Item> = Vec::new();
    resolve_imports_into(
        source,
        base_dir,
        &mut user_items,
        &mut import_errors,
        &stdlib_vfs,
    );

    let combined_items: Vec<_> = prelude_items
        .into_iter()
        .chain(ast_stdlib.items)
        .chain(user_items)
        .collect();
    let combined = SourceFile {
        items: combined_items,
        span: source.span,
    };

    match analyze_inner(&combined) {
        Ok((typed, registry)) if import_errors.is_empty() => Ok((typed, registry)),
        Ok(_) => Err(import_errors),
        Err(mut errs) => {
            import_errors.append(&mut errs);
            Err(import_errors)
        }
    }
}

/// Collect the disk paths of all files transitively imported by `source`.
/// Stdlib/VFS imports are excluded — only real on-disk `.kn` files are returned.
pub fn collect_imported_disk_paths(
    source: &SourceFile,
    base_dir: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    let vfs = crate::stdlib::stdlib_virtual_fs();
    let mut paths = Vec::new();
    collect_import_paths_into(source, base_dir, &vfs, &mut paths);
    paths
}

fn collect_import_paths_into(
    source: &SourceFile,
    base_dir: &std::path::Path,
    vfs: &std::collections::HashMap<String, String>,
    out: &mut Vec<std::path::PathBuf>,
) {
    for item in &source.items {
        if let Item::Import(import) = item {
            let module_key = import.path.join(".");
            if vfs.contains_key(&module_key) {
                continue;
            }
            let rel: std::path::PathBuf = import.path.iter().collect();
            let file_path = base_dir.join(rel).with_extension("kn");
            if !out.contains(&file_path) {
                if let Ok(src) = std::fs::read_to_string(&file_path) {
                    out.push(file_path.clone());
                    let module_base = file_path.parent().unwrap_or(base_dir).to_path_buf();
                    if let Ok(tokens) = crate::lexer::Lexer::new(&src).tokenize() {
                        if let Ok(parsed) = crate::parser::Parser::new(tokens).parse_file() {
                            collect_import_paths_into(&parsed, &module_base, vfs, out);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> SourceFile {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse_file().unwrap()
    }

    #[test]
    fn collect_imported_disk_paths_no_imports_returns_empty() {
        let src = parse("def main() -> void {}");
        let paths = collect_imported_disk_paths(&src, std::path::Path::new("."));
        assert!(paths.is_empty());
    }

    #[test]
    fn collect_imported_disk_paths_stdlib_import_excluded() {
        let src = parse("import prelude { * }");
        let paths = collect_imported_disk_paths(&src, std::path::Path::new("."));
        assert!(
            paths.is_empty(),
            "stdlib imports should be excluded: {paths:?}"
        );
    }

    #[test]
    fn collect_imported_disk_paths_disk_import_included() {
        let src = parse("import modules { * }");
        let paths = collect_imported_disk_paths(&src, std::path::Path::new("examples"));
        assert!(
            paths.iter().any(|p| p.ends_with("modules.kn")),
            "expected modules.kn in paths: {paths:?}"
        );
    }
}

/// Walk all items in `source`, inlining imported items and skipping export blocks.
/// `vfs` is checked before disk for each import path (used for embedded stdlib modules).
fn resolve_imports_into(
    source: &SourceFile,
    base_dir: &std::path::Path,
    out: &mut Vec<Item>,
    errors: &mut Vec<AnalysisError>,
    vfs: &std::collections::HashMap<String, String>,
) {
    for item in &source.items {
        match item {
            Item::Import(import) => {
                resolve_one_import(import, base_dir, out, errors, vfs);
            }
            Item::Export(_) => {
                // Export blocks are metadata consumed by importers; skip here.
            }
            other => out.push(other.clone()),
        }
    }
}

fn resolve_one_import(
    import: &crate::parser::ast::Import,
    base_dir: &std::path::Path,
    out: &mut Vec<Item>,
    errors: &mut Vec<AnalysisError>,
    vfs: &std::collections::HashMap<String, String>,
) {
    let module_key = import.path.join(".");

    // Check the virtual filesystem first (embedded stdlib modules).
    let (src, disk_path): (String, Option<std::path::PathBuf>) =
        if let Some(embedded) = vfs.get(&module_key) {
            (embedded.clone(), None)
        } else {
            // Fall back to disk.
            let rel: std::path::PathBuf = import.path.iter().collect();
            let file_path = base_dir.join(rel).with_extension("kn");
            match std::fs::read_to_string(&file_path) {
                Ok(s) => (s, Some(file_path)),
                Err(_) => {
                    errors.push(AnalysisError::ModuleNotFound {
                        path: module_key,
                        span: import.span,
                    });
                    return;
                }
            }
        };

    let module_base: &std::path::Path = disk_path
        .as_ref()
        .and_then(|p| p.parent())
        .unwrap_or(base_dir);

    let tokens = match crate::lexer::Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(_) => {
            errors.push(AnalysisError::ModuleNotFound {
                path: import.path.join("."),
                span: import.span,
            });
            return;
        }
    };
    let parsed = match crate::parser::Parser::new(tokens).parse_file() {
        Ok(f) => f,
        Err(_) => {
            errors.push(AnalysisError::ModuleNotFound {
                path: import.path.join("."),
                span: import.span,
            });
            return;
        }
    };

    // Recursively resolve imports inside the imported module (vfs threads through).
    let mut module_items: Vec<Item> = Vec::new();
    resolve_imports_into(&parsed, module_base, &mut module_items, errors, vfs);

    // Collect the set of exported symbol names for this file.
    let exported: std::collections::HashSet<String> = parsed
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Export(e) = i {
                Some(&e.symbols)
            } else {
                None
            }
        })
        .flatten()
        .cloned()
        .collect();

    let wildcard_export = exported.contains("*");
    let wildcard_import = import.symbols.iter().any(|s| s == "*");

    // Filter module items to only those the module exports.
    // Nameless items (impl blocks) are included when the module uses export { * }.
    let exported_items: Vec<Item> = module_items
        .into_iter()
        .filter(|i| match item_top_name(i) {
            Some(n) => wildcard_export || exported.contains(n),
            None => wildcard_export,
        })
        .collect();

    if wildcard_import {
        // Bring in all exported items.
        out.extend(exported_items);
    } else {
        // Selective import: bring in only the requested symbols, error on missing.
        let module_name = import.path.join(".");
        for sym in &import.symbols {
            let found = exported_items
                .iter()
                .find(|i| item_top_name(i) == Some(sym.as_str()));
            match found {
                Some(i) => out.push(i.clone()),
                None => {
                    errors.push(AnalysisError::SymbolNotExported {
                        symbol: sym.clone(),
                        module: module_name.clone(),
                        span: import.span,
                    });
                }
            }
        }
    }
}

/// Return the top-level name of an item, if it has one (functions, structs, enums, interfaces,
/// type aliases, globals). Returns None for impl blocks and other nameless items.
fn item_top_name(item: &Item) -> Option<&str> {
    match item {
        Item::Function(f) => Some(&f.name),
        Item::Struct(s) => Some(&s.name),
        Item::Enum(e) => Some(&e.name),
        Item::Interface(i) => Some(&i.name),
        Item::TypeAlias(t) => Some(&t.name),
        Item::Global(g) => Some(&g.name),
        _ => None,
    }
}

fn analyze_inner(source: &SourceFile) -> Result<(TypedFile, ty::TypeRegistry), Vec<AnalysisError>> {
    let mut errors: Vec<AnalysisError> = Vec::new();
    let mut env = Env::new();
    let mut registry = ty::TypeRegistry::new();

    env.push_scope();
    register_builtins(&mut env, &mut registry);

    // Pass 1: collect top-level names.
    errors.extend(collect::collect_top_level(source, &mut env, &mut registry));

    // Deprecation warnings: scan all non-deprecated function bodies for calls
    // to @deprecated functions and emit warnings to stderr.
    {
        let deprecated: std::collections::HashSet<String> = source
            .items
            .iter()
            .filter_map(|item| {
                if let Item::Function(f) = item {
                    if f.annotations.iter().any(|a| a.name == "deprecated") {
                        Some(f.name.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        if !deprecated.is_empty() {
            for item in &source.items {
                if let Item::Function(f) = item {
                    if f.annotations.iter().any(|a| a.name == "deprecated") {
                        continue;
                    }
                    warn_deprecated_in_block(&f.body, &deprecated, &f.name);
                }
            }
        }
    }

    // Pass 1a: register module-level globals and consts into the environment.
    for item in &source.items {
        match item {
            Item::Global(g) => {
                let ty = resolve::resolve_type_expr(&g.ty, &env, &mut errors);
                env.define(
                    &g.name,
                    env::Symbol::Var {
                        ty,
                        mutable: g.mutable,
                        span: g.span,
                    },
                );
            }
            Item::Const(c) => {
                let ty = resolve::resolve_type_expr(&c.ty, &env, &mut errors);
                // Const initializer must be a literal.
                let value_kind = match &c.value {
                    Expr::Int(n, _) => Some(crate::analyzer::typed_ast::TypedExprKind::Int(*n)),
                    Expr::Float(f, _) => Some(crate::analyzer::typed_ast::TypedExprKind::Float(*f)),
                    Expr::Bool(b, _) => Some(crate::analyzer::typed_ast::TypedExprKind::Bool(*b)),
                    Expr::Str(segs, _) => {
                        // Only allow all-text string literals.
                        let text_segs: Option<Vec<_>> = segs
                            .iter()
                            .map(|seg| match seg {
                                StringSegment::Text(t) => Some(
                                    crate::analyzer::typed_ast::TypedStringSegment::Text(t.clone()),
                                ),
                                _ => None,
                            })
                            .collect();
                        text_segs.map(crate::analyzer::typed_ast::TypedExprKind::Str)
                    }
                    _ => None,
                };
                match value_kind {
                    Some(kind) => {
                        env.define(
                            &c.name,
                            env::Symbol::Const {
                                ty,
                                value: kind,
                                span: c.span,
                            },
                        );
                    }
                    None => {
                        errors.push(AnalysisError::NonLiteralConst {
                            name: c.name.clone(),
                            span: c.span,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Pass 1a2: check that all annotation uses name a declared or built-in annotation.
    {
        const BUILTIN_ANNOTATIONS: &[&str] = &[
            "static",
            "builtin",
            "deprecated",
            "inline",
            "indirect",
            "derive",
            "test",
            "entry",
            "impure",
        ];
        let declared: std::collections::HashSet<String> = source
            .items
            .iter()
            .filter_map(|i| {
                if let Item::AnnotationDef(a) = i {
                    Some(a.name.clone())
                } else {
                    None
                }
            })
            .collect();
        for item in &source.items {
            let anns: &[crate::parser::ast::AnnotationUse] = match item {
                Item::Function(f) => &f.annotations,
                Item::Struct(s) => &s.annotations,
                Item::Enum(e) => &e.annotations,
                _ => &[],
            };
            for ann in anns {
                if !BUILTIN_ANNOTATIONS.contains(&ann.name.as_str())
                    && !declared.contains(&ann.name)
                {
                    errors.push(error::AnalysisError::UnknownAnnotation {
                        name: ann.name.clone(),
                        span: ann.span,
                    });
                }
            }
        }
    }

    // Pass 1b: resolve top-level function signatures, grouping overloads.
    {
        // Collect function items grouped by name, preserving first-appearance order.
        use std::collections::HashMap;
        let mut fn_order: Vec<String> = Vec::new();
        let mut fn_groups: HashMap<String, Vec<usize>> = HashMap::new();
        let fns: Vec<_> = source
            .items
            .iter()
            .filter_map(|i| {
                if let Item::Function(f) = i {
                    Some(f)
                } else {
                    None
                }
            })
            .collect();
        for (idx, f) in fns.iter().enumerate() {
            if !fn_groups.contains_key(&f.name) {
                fn_order.push(f.name.clone());
            }
            fn_groups.entry(f.name.clone()).or_default().push(idx);
        }

        for name in &fn_order {
            let indices = &fn_groups[name];
            if indices.len() == 1 {
                let f = fns[indices[0]];
                let has_generics = !f.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &f.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                    register_projection_pins(&f.generic_params, &mut env, &mut errors);
                }
                let ret = resolve_type_expr(&f.return_type, &env, &mut errors);
                let params: Vec<(String, Ty)> = f
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                    .collect();
                let generic_bounds = build_generic_bounds(&f.generic_params, &env, &mut errors);
                if has_generics {
                    env.pop_scope();
                }
                let generic_param_names: Vec<String> =
                    f.generic_params.iter().map(|g| g.name.clone()).collect();

                // @builtin def with no params is a value-level declaration (a nullary
                // constructor). Register it as Symbol::Var so that bare references
                // resolve to the value type rather than a Callable type. Generic params
                // are replaced with Unknown so the type unifier can infer them from
                // context (e.g. `x: Option[int] = None` infers None: Option[int]).
                let is_builtin = f.annotations.iter().any(|a| a.name == "builtin");
                // A lone non-builtin declaration with no implementation is an error.
                if f.is_declaration && !is_builtin {
                    errors.push(error::AnalysisError::MissingImplementation {
                        name: name.clone(),
                        span: f.span,
                    });
                }
                if is_builtin && params.is_empty() {
                    let value_ty = subst_generic_params_unknown(&ret, &generic_param_names);
                    env.define(
                        name,
                        Symbol::Var {
                            ty: value_ty,
                            mutable: false,
                            span: f.span,
                        },
                    );
                } else {
                    env.define(
                        name,
                        Symbol::Fn {
                            generic_params: generic_param_names,
                            generic_bounds,
                            inferred_bounds: vec![],
                            params,
                            ret,
                            span: f.span,
                        },
                    );
                }
            } else {
                // Resolve all functions in this name group.
                struct ResolvedFn<'a> {
                    f: &'a crate::parser::ast::FnDef,
                    params: Vec<(String, Ty)>,
                    ret: Ty,
                    generic_bounds: Vec<GenericBound>,
                    generic_param_names: Vec<String>,
                    is_builtin: bool,
                }
                let mut resolved: Vec<ResolvedFn> = Vec::new();
                for &global_idx in indices.iter() {
                    let f = fns[global_idx];
                    let has_generics = !f.generic_params.is_empty();
                    if has_generics {
                        env.push_scope();
                        for gp in &f.generic_params {
                            env.define(
                                &gp.name,
                                Symbol::Type {
                                    id: TypeId(0),
                                    span: gp.span,
                                },
                            );
                        }
                        register_projection_pins(&f.generic_params, &mut env, &mut errors);
                    }
                    let ret = resolve_type_expr(&f.return_type, &env, &mut errors);
                    let params: Vec<(String, Ty)> = f
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                        .collect();
                    let generic_bounds = build_generic_bounds(&f.generic_params, &env, &mut errors);
                    if has_generics {
                        env.pop_scope();
                    }
                    let generic_param_names: Vec<String> =
                        f.generic_params.iter().map(|g| g.name.clone()).collect();
                    let is_builtin = f.annotations.iter().any(|a| a.name == "builtin");
                    resolved.push(ResolvedFn {
                        f,
                        params,
                        ret,
                        generic_bounds,
                        generic_param_names,
                        is_builtin,
                    });
                }

                // Separate declarations from implementations.
                // Builtins are always kept as declarations; their "impl" is in codegen.
                let decl_indices: Vec<usize> = resolved
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.f.is_declaration)
                    .map(|(i, _)| i)
                    .collect();
                let impl_indices: Vec<usize> = resolved
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| !r.f.is_declaration)
                    .map(|(i, _)| i)
                    .collect();

                // Check for duplicate signatures within declarations (E016).
                for i in 0..decl_indices.len() {
                    for j in (i + 1)..decl_indices.len() {
                        let di = decl_indices[i];
                        let dj = decl_indices[j];
                        if params_are_duplicate(&resolved[di].params, &resolved[dj].params) {
                            errors.push(error::AnalysisError::DuplicateSignature {
                                name: name.clone(),
                                span: resolved[dj].f.span,
                            });
                        }
                    }
                }

                // Check for duplicate signatures within implementations (E016).
                for i in 0..impl_indices.len() {
                    for j in (i + 1)..impl_indices.len() {
                        let ii = impl_indices[i];
                        let ij = impl_indices[j];
                        if params_are_duplicate(&resolved[ii].params, &resolved[ij].params) {
                            errors.push(error::AnalysisError::DuplicateSignature {
                                name: name.clone(),
                                span: resolved[ij].f.span,
                            });
                        }
                    }
                }

                // Check for orphan declarations: non-builtin declarations with no
                // matching implementation (same arity) are an error (E018).
                for &di in &decl_indices {
                    if resolved[di].is_builtin {
                        continue;
                    }
                    let has_impl = impl_indices
                        .iter()
                        .any(|&ii| resolved[ii].params.len() == resolved[di].params.len());
                    if !has_impl {
                        errors.push(error::AnalysisError::MissingImplementation {
                            name: name.clone(),
                            span: resolved[di].f.span,
                        });
                    }
                }

                // Build overload entries for implementations only.
                // For each implementation, check if there is a matching declaration
                // (same arity) and inherit its bounds.
                let mut overloads: Vec<FnOverload> = Vec::new();
                let mut overload_local_idx = 0usize;
                for &ii in &impl_indices {
                    let mangled_name = format!("{}__{}", name, overload_local_idx);
                    overload_local_idx += 1;

                    // Find a matching declaration (non-builtin) with same arity.
                    let matching_decl = decl_indices.iter().find(|&&di| {
                        !resolved[di].is_builtin
                            && resolved[di].params.len() == resolved[ii].params.len()
                    });

                    // If a matching declaration exists, the implementation must not carry
                    // its own bounds -- they belong canonically on the declaration.
                    if matching_decl.is_some() && !resolved[ii].generic_bounds.is_empty() {
                        errors.push(error::AnalysisError::BoundsOnImplementation {
                            name: name.clone(),
                            span: resolved[ii].f.span,
                        });
                    }

                    let generic_bounds = if let Some(&di) = matching_decl {
                        // Inherit bounds from the declaration.
                        resolved[di].generic_bounds.clone()
                    } else {
                        resolved[ii].generic_bounds.clone()
                    };

                    overloads.push(FnOverload {
                        generic_params: resolved[ii].generic_param_names.clone(),
                        generic_bounds,
                        inferred_bounds: vec![],
                        params: resolved[ii].params.clone(),
                        ret: resolved[ii].ret.clone(),
                        mangled_name,
                        span: resolved[ii].f.span,
                    });
                }

                // If there are builtin declarations (like @builtin def None), also add them
                // to the overload set so they resolve correctly as Symbol::Var or callable.
                let builtin_decl_indices: Vec<usize> = decl_indices
                    .iter()
                    .filter(|&&di| resolved[di].is_builtin)
                    .copied()
                    .collect();
                for &di in &builtin_decl_indices {
                    let mangled_name = format!("{}__{}", name, overload_local_idx);
                    overload_local_idx += 1;
                    overloads.push(FnOverload {
                        generic_params: resolved[di].generic_param_names.clone(),
                        generic_bounds: resolved[di].generic_bounds.clone(),
                        inferred_bounds: vec![],
                        params: resolved[di].params.clone(),
                        ret: resolved[di].ret.clone(),
                        mangled_name,
                        span: resolved[di].f.span,
                    });
                }

                env.define(name, Symbol::FnOverloadSet { overloads });
            }
        }
    }

    // Pass 1c: register struct fields (and builtin method decls) into the registry.
    for item in &source.items {
        if let Item::Struct(s) = item {
            if s.is_builtin {
                // Builtin structs declare their methods as FnDecl entries.
                // Register them so the analyzer can type-check calls, using the
                // real source spans from the prelude file.
                let has_generics = !s.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &s.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                for decl in &s.decls {
                    let params: Vec<(String, Ty)> = decl
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                        .collect();
                    let ret = resolve_type_expr(&decl.return_type, &env, &mut errors);
                    let qualified_fn = format!("{}_{}", s.name, decl.name);
                    registry.register_method(
                        &s.name,
                        ty::MethodEntry {
                            method_name: decl.name.clone(),
                            qualified_fn,
                            params,
                            ret,
                        },
                    );
                }
                let fields: Vec<(String, Ty)> = s
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_type_expr(&f.ty, &env, &mut errors)))
                    .collect();
                if !fields.is_empty() {
                    registry.register_struct_fields(&s.name, fields);
                }
                if has_generics {
                    env.pop_scope();
                }
            } else {
                let has_generics = !s.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &s.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                    let param_names: Vec<String> =
                        s.generic_params.iter().map(|g| g.name.clone()).collect();
                    registry.register_generic_param_order(&s.name, param_names);
                }
                let fields: Vec<(String, Ty)> = s
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_type_expr(&f.ty, &env, &mut errors)))
                    .collect();
                registry.register_struct_fields(&s.name, fields);
                // Register inline method signatures for type-checking call sites.
                for method in &s.methods {
                    let params: Vec<(String, Ty)> = method
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                        .collect();
                    let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                    let qualified_fn = format!("{}_{}", s.name, method.name);
                    registry.register_method(
                        &s.name,
                        ty::MethodEntry {
                            method_name: method.name.clone(),
                            qualified_fn,
                            params,
                            ret,
                        },
                    );
                }
                if has_generics {
                    env.pop_scope();
                }
            }
        }
    }

    // Fix 6b: detect recursive struct types that lack @indirect on the self-referential field.
    // A struct like `struct Node { next: Node }` has infinite size; the field must carry @indirect.
    for item in &source.items {
        if let Item::Struct(s) = item {
            if s.is_builtin {
                continue;
            }
            for field in &s.fields {
                let is_indirect = field.annotations.iter().any(|a| a.name == "indirect");
                if is_indirect {
                    continue;
                }
                // Check if the field's type directly names this struct (ignoring generics).
                let refers_to_self = match &field.ty {
                    TypeExpr::Named { name, .. } => name == &s.name,
                    _ => false,
                };
                if refers_to_self {
                    errors.push(AnalysisError::RecursiveTypeWithoutIndirect {
                        ty: s.name.clone(),
                        field: field.name.clone(),
                        span: field.span,
                    });
                }
            }
        }
    }

    // Pass 1c2: register enum variant fields and generic param order so match arm
    // bindings get properly-typed variables (e.g. `v: int` in `Some { value: v }`
    // when the scrutinee is `Option[int]`).
    for item in &source.items {
        if let Item::Enum(e) = item {
            let has_generics = !e.generic_params.is_empty();
            if has_generics {
                env.push_scope();
                for gp in &e.generic_params {
                    env.define(
                        &gp.name,
                        Symbol::Type {
                            id: TypeId(0),
                            span: gp.span,
                        },
                    );
                }
                let param_names: Vec<String> =
                    e.generic_params.iter().map(|g| g.name.clone()).collect();
                registry.register_generic_param_order(&e.name, param_names);
            }
            for variant in &e.variants {
                if !variant.fields.is_empty() {
                    let fields: Vec<(String, Ty)> = variant
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), resolve_type_expr(&f.ty, &env, &mut errors)))
                        .collect();
                    registry.register_struct_fields(&variant.name, fields);
                }
            }
            if has_generics {
                env.pop_scope();
            }
        }
    }

    // Pass 1d: register impl block methods and conformances into the registry.
    for item in &source.items {
        if let Item::ImplBlock(impl_block) = item {
            let type_name = match &impl_block.for_type {
                TypeExpr::Named { name, .. } => name.clone(),
                _ => continue,
            };
            let iface_name = match &impl_block.interface {
                TypeExpr::Named { name, .. } => name.clone(),
                _ => String::new(),
            };

            // Push scope for generic params before resolving assoc bindings or types.
            // Prefer explicit impl-level params; fall back to extracting from for_type generics.
            let scope_params: Vec<String> = if !impl_block.generic_params.is_empty() {
                impl_block
                    .generic_params
                    .iter()
                    .map(|g| g.name.clone())
                    .collect()
            } else {
                match &impl_block.for_type {
                    TypeExpr::Named { generics, .. } => generics
                        .iter()
                        .filter_map(|g| {
                            if let TypeExpr::Named { name, .. } = g {
                                Some(name.clone())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => vec![],
                }
            };
            let has_generics = !scope_params.is_empty();
            let dummy_span = impl_block.span;
            if has_generics {
                env.push_scope();
                for gname in &scope_params {
                    env.define(
                        gname,
                        Symbol::Type {
                            id: TypeId(0),
                            span: dummy_span,
                        },
                    );
                }
            }
            // Register conformance entry. Bounds come from impl-level generic params.
            // Assoc bindings are resolved with generic params in scope.
            if !iface_name.is_empty() {
                let bounds: Vec<(String, String)> = impl_block
                    .generic_params
                    .iter()
                    .flat_map(|gp| {
                        gp.bounds.iter().filter_map(move |b| {
                            if let TypeExpr::Named { name, .. } = b {
                                Some((gp.name.clone(), name.clone()))
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                let bindings: Vec<(String, ty::Ty)> = impl_block
                    .assoc_bindings
                    .iter()
                    .map(|(name, ty_expr)| {
                        (name.clone(), resolve_type_expr(ty_expr, &env, &mut errors))
                    })
                    .collect();
                registry.register_conformance(
                    &type_name,
                    &iface_name,
                    ty::ConformanceEntry { bounds, bindings },
                );
            }

            // Define `Self` and any user alias so hook/method signatures can use them.
            // Look up the concrete type from the registry so Self resolves to Ty::Named("Item")
            // rather than a generic param. Primitive types (int, bool, float, str) are not in
            // the registry by name, so fall back to resolve_type_expr on the for_type.
            let self_concrete_ty = if let Some(e) = registry.lookup_by_name(&type_name) {
                Ty::Named(e.id.clone(), type_name.clone(), vec![])
            } else {
                resolve_type_expr(&impl_block.for_type, &env, &mut errors)
            };
            env.push_scope();
            env.define("Self", Symbol::TypeAlias(self_concrete_ty.clone()));
            if let Some(alias) = &impl_block.self_alias {
                env.define(alias, Symbol::TypeAlias(self_concrete_ty));
            }
            for method in &impl_block.methods {
                let params: Vec<(String, Ty)> = method
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                    .collect();
                let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                let qualified_fn = format!("{}_{}", type_name, method.name);
                registry.register_method(
                    &type_name,
                    MethodEntry {
                        method_name: method.name.clone(),
                        qualified_fn,
                        params,
                        ret,
                    },
                );
            }
            for hook in &impl_block.hooks {
                let hook_name = match &hook.name {
                    HookName::Named(n) => n.clone(),
                    HookName::Op(op) => op.clone(),
                };
                let params: Vec<(String, Ty)> = hook
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                    .collect();
                let ret = hook
                    .return_type
                    .as_ref()
                    .map(|r| resolve_type_expr(r, &env, &mut errors))
                    .unwrap_or(Ty::Void);
                let qualified_fn = format!("{}__hook__{}", type_name, hook_name);
                registry.register_method(
                    &type_name,
                    MethodEntry {
                        method_name: hook_name,
                        qualified_fn,
                        params,
                        ret,
                    },
                );
            }
            env.pop_scope(); // Self + alias scope
            if has_generics {
                env.pop_scope();
            }
        }
    }

    let interfaces: Vec<_> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Interface(iface) = i {
                Some(iface.clone())
            } else {
                None
            }
        })
        .collect();

    let all_impls: Vec<_> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::ImplBlock(b) = i {
                Some(b.clone())
            } else {
                None
            }
        })
        .collect();

    // Pass 1d2: auto-register Display conformance for all enums.
    // Every enum automatically implements Display, returning the variant name.
    for item in &source.items {
        if let Item::Enum(e) = item {
            registry.register_conformance(
                &e.name,
                "Display",
                ty::ConformanceEntry {
                    bounds: vec![],
                    bindings: vec![],
                },
            );
        }
    }

    // Pass 1e: register interface method/hook signatures for use by infer.rs.
    for item in &source.items {
        if let Item::Interface(iface) = item {
            use crate::parser::ast::InterfaceItemKind;
            // Push Self, generic params, and associated types so that hook/method
            // signatures using them (e.g. `-> Output`, `rhs: Self`) resolve cleanly.
            env.push_scope();
            let dummy_span = iface.span;
            env.define(
                "Self",
                Symbol::Type {
                    id: TypeId(0),
                    span: dummy_span,
                },
            );
            for gp in &iface.generic_params {
                env.define(
                    &gp.name,
                    Symbol::Type {
                        id: TypeId(0),
                        span: gp.span,
                    },
                );
            }
            for iitem in &iface.items {
                if let InterfaceItemKind::AssocType { name, .. } = &iitem.kind {
                    env.define(
                        name,
                        Symbol::Type {
                            id: TypeId(0),
                            span: iitem.span,
                        },
                    );
                    registry.register_iface_assoc_type(&iface.name, name);
                }
            }
            for iitem in &iface.items {
                match &iitem.kind {
                    InterfaceItemKind::Method(method) => {
                        let has_method_generics = !method.generic_params.is_empty();
                        if has_method_generics {
                            env.push_scope();
                            for gp in &method.generic_params {
                                env.define(
                                    &gp.name,
                                    Symbol::Type {
                                        id: TypeId(0),
                                        span: gp.span,
                                    },
                                );
                            }
                        }
                        let params: Vec<(String, Ty)> = method
                            .params
                            .iter()
                            .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                            .collect();
                        let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                        if has_method_generics {
                            env.pop_scope();
                        }
                        registry.register_interface_method(
                            &iface.name,
                            MethodEntry {
                                method_name: method.name.clone(),
                                qualified_fn: method.name.clone(),
                                params,
                                ret,
                            },
                        );
                    }
                    InterfaceItemKind::Hook {
                        name,
                        params,
                        return_type,
                        ..
                    } => {
                        let hook_name = match name {
                            HookName::Named(n) => n.clone(),
                            HookName::Op(op) => op.clone(),
                        };
                        let resolved_params: Vec<(String, Ty)> = params
                            .iter()
                            .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                            .collect();
                        let ret = return_type
                            .as_ref()
                            .map(|r| resolve_type_expr(r, &env, &mut errors))
                            .unwrap_or(Ty::Void);
                        registry.register_interface_method(
                            &iface.name,
                            MethodEntry {
                                method_name: hook_name.clone(),
                                qualified_fn: hook_name,
                                params: resolved_params,
                                ret,
                            },
                        );
                    }
                    InterfaceItemKind::Field { .. } => {}
                    InterfaceItemKind::AssocType { .. } => {}
                }
            }
            // Register direct superinterfaces for implication reasoning.
            let supers: Vec<String> = iface
                .extends
                .iter()
                .filter_map(|t| {
                    if let crate::parser::ast::TypeExpr::Named { name, .. } = t {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if !supers.is_empty() {
                registry.register_interface_supers(&iface.name, supers);
            }
            env.pop_scope();
        }
    }

    // Precompute the transitive superinterface closure after all interfaces are registered.
    // Converts iface_implies from per-query BFS to O(1) set lookup for constraint solving.
    registry.precompute_transitive_closures();

    // Fix 6a: detect cyclic interface hierarchies (interface A extends B extends A).
    // After precompute, a cycle exists iff an interface is in its own transitive closure.
    for item in &source.items {
        if let Item::Interface(iface) = item {
            if registry.iface_implies(&iface.name, &iface.name)
                && registry
                    .get_transitive_supers(&iface.name)
                    .map(|s| s.contains(&iface.name))
                    .unwrap_or(false)
            {
                // Build a human-readable cycle string: A -> B -> A
                let cycle = format!("{} -> ... -> {}", iface.name, iface.name);
                errors.push(AnalysisError::CyclicInterface {
                    iface: iface.name.clone(),
                    cycle,
                    span: iface.span,
                });
            }
        }
    }

    // Pass 1e2: conformance propagation.
    // For every direct (type, iface) conformance, also register it for every transitive
    // superinterface of iface so that constraint solving never needs to traverse the
    // superinterface graph at query time.
    {
        let direct = registry.all_direct_conformances();
        for (type_name, iface_name, entries) in direct {
            let supers: Vec<String> = registry
                .get_transitive_supers(&iface_name)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            for super_iface in supers {
                // Only add if not already registered to avoid duplicates.
                if registry
                    .get_conformances(&type_name, &super_iface)
                    .is_empty()
                {
                    for e in &entries {
                        registry.register_conformance(&type_name, &super_iface, e.clone());
                    }
                }
            }
        }
    }

    // Pass 1f: compute variance for all generic type parameters.
    //
    // For each struct and builtin struct with generic params:
    //   1. Register the param name order.
    //   2. Infer variance from method signatures: output-only => covariant,
    //      input-only => contravariant, both => invariant, neither => bivariant.
    //   3. Apply any explicit variance annotation (+T/-T) from the AST, overriding
    //      the inferred result.
    //
    // After inference, override hardcoded invariant types (Mutex[T]).
    {
        use crate::analyzer::ty::ComputedVariance;
        use crate::parser::ast::Variance;

        fn variance_of_ty(ty: &Ty, param: &str) -> (bool, bool) {
            // Returns (appears_in_output, appears_in_input)
            match ty {
                Ty::GenericParam(n) if n == param => (true, false),
                Ty::Named(_, _, args) => args
                    .iter()
                    .map(|a| variance_of_ty(a, param))
                    .fold((false, false), |(ao, ai), (o, i)| (ao || o, ai || i)),
                Ty::Tuple(ts) => ts
                    .iter()
                    .map(|t| variance_of_ty(t, param))
                    .fold((false, false), |(ao, ai), (o, i)| (ao || o, ai || i)),
                Ty::Callable(params, ret) => {
                    // Callable is contravariant in params, covariant in ret.
                    let (_, in_params) = params
                        .iter()
                        .map(|p| variance_of_ty(p, param))
                        .fold((false, false), |(ao, ai), (o, i)| (ao || o, ai || i));
                    let (in_ret, _) = variance_of_ty(ret, param);
                    (in_ret, in_params)
                }
                Ty::Ref(inner, _) => variance_of_ty(inner, param),
                Ty::Union(ts) | Ty::Compound(ts) => ts
                    .iter()
                    .map(|t| variance_of_ty(t, param))
                    .fold((false, false), |(ao, ai), (o, i)| (ao || o, ai || i)),
                _ => (false, false),
            }
        }

        for item in &source.items {
            let (type_name, generic_params) = match item {
                Item::Struct(s) if !s.generic_params.is_empty() => (&s.name, &s.generic_params),
                _ => continue,
            };

            let param_names: Vec<String> = generic_params.iter().map(|g| g.name.clone()).collect();
            registry.register_generic_param_order(type_name, param_names.clone());

            // Look up all registered methods for this type.
            for (idx, gp) in generic_params.iter().enumerate() {
                // If the user declared an explicit variance annotation, use it.
                let explicit = match gp.variance {
                    Variance::Covariant => Some(ComputedVariance::Covariant),
                    Variance::Contravariant => Some(ComputedVariance::Contravariant),
                    Variance::Invariant => None, // default; infer from signatures
                };
                if let Some(v) = explicit {
                    registry.register_type_variance(type_name, idx, v);
                    continue;
                }

                // Infer from method signatures.
                let mut combined = ComputedVariance::Bivariant;
                // Look at the struct's fields for variance hints.
                if let Some(fields) = registry.get_struct_fields(type_name) {
                    let fields: Vec<_> = fields.to_vec();
                    for (_, fty) in &fields {
                        let (in_out, in_in) = variance_of_ty(fty, &gp.name);
                        let pos_variance = match (in_out, in_in) {
                            (true, true) => ComputedVariance::Invariant,
                            (true, false) => ComputedVariance::Covariant,
                            (false, true) => ComputedVariance::Contravariant,
                            (false, false) => ComputedVariance::Bivariant,
                        };
                        combined = combined.combine(pos_variance);
                    }
                }
                registry.register_type_variance(type_name, idx, combined);
            }
        }

        // Hardcoded invariant overrides for types with interior mutability.
        // Mutex[T] must be invariant in T regardless of what method signatures say.
        if let Some(order) = registry
            .get_generic_param_order("Mutex")
            .map(|o| o.to_vec())
        {
            for (idx, _) in order.iter().enumerate() {
                registry.register_type_variance("Mutex", idx, ComputedVariance::Invariant);
            }
        }
    }

    // Pass 2: check each item and produce typed items.
    let mut typed_items: Vec<TypedItem> = Vec::new();
    let mut plain_impls_seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for item in &source.items {
        match item {
            Item::Function(f) => {
                // Skip non-builtin body-less declarations -- no body to type-check.
                // @builtin declarations are emitted as TypedFnDef for codegen.
                let is_builtin = f.annotations.iter().any(|a| a.name == "builtin");
                if f.is_declaration && !is_builtin {
                    continue;
                }

                // Determine the mangled name: bare name for single-def functions,
                // "name__N" for overloads.
                let mangled_name = match env.lookup(&f.name) {
                    Some(Symbol::FnOverloadSet { overloads }) => overloads
                        .iter()
                        .find(|o| o.span == f.span)
                        .map(|o| o.mangled_name.clone())
                        .unwrap_or(f.name.clone()),
                    _ => f.name.clone(),
                };

                let has_generics = !f.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &f.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                let ret = resolve_type_expr(&f.return_type, &env, &mut errors);
                let mut params: Vec<TypedParam> = Vec::new();
                env.push_scope();
                for p in &f.params {
                    let pty = resolve_type_expr(&p.ty, &env, &mut errors);
                    env.define(
                        &p.name,
                        Symbol::Var {
                            ty: pty.clone(),
                            mutable: p.mutable,
                            span: p.span,
                        },
                    );
                    params.push(TypedParam {
                        name: p.name.clone(),
                        ty: pty,
                        mutable: p.mutable,
                        span: p.span,
                    });
                }
                let body =
                    check::check_typed_block(&f.body, &mut env, &registry, &ret, &mut errors);
                env.pop_scope();
                if has_generics {
                    env.pop_scope();
                    // Infer bounds from how the generic params are used in the body.
                    let gparams: Vec<String> =
                        f.generic_params.iter().map(|g| g.name.clone()).collect();
                    let inferred = infer_bounds::infer_bounds_from_body(&body, &gparams, &registry);
                    if !inferred.is_empty() {
                        match env.lookup_mut(&f.name) {
                            Some(Symbol::Fn {
                                ref mut inferred_bounds,
                                ..
                            }) => {
                                *inferred_bounds = inferred;
                            }
                            Some(Symbol::FnOverloadSet { ref mut overloads }) => {
                                if let Some(ov) = overloads.iter_mut().find(|o| o.span == f.span) {
                                    ov.inferred_bounds = inferred;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if ret != Ty::Void && !f.body.stmts.is_empty() && !returns::always_returns(&f.body)
                {
                    errors.push(AnalysisError::MissingReturn {
                        name: f.name.clone(),
                        span: f.span,
                    });
                }
                typed_items.push(TypedItem::Function(TypedFnDef {
                    name: mangled_name,
                    params,
                    variadic: f.variadic.as_ref().map(|v| v.name.clone()),
                    return_type: ret,
                    body,
                    is_builtin: f.annotations.iter().any(|a| a.name == "builtin"),
                    is_inline: f.annotations.iter().any(|a| a.name == "inline"),
                    is_declaration: f.is_declaration,
                    is_entry: f.annotations.iter().any(|a| a.name == "entry"),
                    is_impure: f.annotations.iter().any(|a| a.name == "impure"),
                    span: f.span,
                }));
            }

            Item::Struct(s) => {
                if !s.is_builtin {
                    conformance::check_struct_conformance(s, &interfaces, &all_impls, &mut errors);
                }
                let has_struct_generics = !s.generic_params.is_empty();
                if has_struct_generics {
                    env.push_scope();
                    for gp in &s.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                let fields: Vec<TypedField> = s
                    .fields
                    .iter()
                    .map(|f| TypedField {
                        name: f.name.clone(),
                        ty: resolve_type_expr(&f.ty, &env, &mut errors),
                        is_priv: f.is_priv,
                        is_indirect: f.annotations.iter().any(|a| a.name == "indirect"),
                        span: f.span,
                    })
                    .collect();
                if has_struct_generics {
                    env.pop_scope();
                }
                typed_items.push(TypedItem::Struct(TypedStructDef {
                    name: s.name.clone(),
                    is_builtin: s.is_builtin,
                    fields,
                    span: s.span,
                }));

                // Emit TypedItem::Function for each inline method body.
                if !s.methods.is_empty() {
                    // Reconstruct self_ty: Named(id, name, [GenericParam(p) for each param]).
                    let type_id = registry
                        .lookup_by_name(&s.name)
                        .map(|e| e.id.clone())
                        .unwrap_or(ty::TypeId(0));
                    let self_ty = if s.generic_params.is_empty() {
                        ty::Ty::Named(type_id, s.name.clone(), vec![])
                    } else {
                        ty::Ty::Named(
                            type_id,
                            s.name.clone(),
                            s.generic_params
                                .iter()
                                .map(|gp| ty::Ty::GenericParam(gp.name.clone()))
                                .collect(),
                        )
                    };

                    // Push generic param scope.
                    if has_struct_generics {
                        env.push_scope();
                        for gp in &s.generic_params {
                            env.define(
                                &gp.name,
                                Symbol::Type {
                                    id: ty::TypeId(0),
                                    span: gp.span,
                                },
                            );
                        }
                    }

                    let struct_fields: Vec<(String, ty::Ty)> = registry
                        .get_struct_fields(&s.name)
                        .map(|fs| fs.to_vec())
                        .unwrap_or_default();

                    for method in &s.methods {
                        env.push_scope();
                        env.define(
                            "self",
                            Symbol::Var {
                                ty: self_ty.clone(),
                                mutable: false,
                                span: method.span,
                            },
                        );
                        env.define("Self", Symbol::TypeAlias(self_ty.clone()));
                        for (fname, fty) in &struct_fields {
                            env.define(fname, Symbol::StructField { ty: fty.clone() });
                        }
                        let ret_raw =
                            resolve::resolve_type_expr(&method.return_type, &env, &mut errors);
                        // Erase generic params in the ABI signature so is_generic_fn returns
                        // false: all Kiln values are i64 at codegen, so one version works for
                        // all T.  The body retains GenericParam types for correct type-checking.
                        let gp_names: Vec<String> =
                            s.generic_params.iter().map(|gp| gp.name.clone()).collect();
                        let ret = subst_generic_params_unknown(&ret_raw, &gp_names);
                        let self_ty_erased = subst_generic_params_unknown(&self_ty, &gp_names);
                        // Prepend __self so codegen has a pointer to the struct in slot 0.
                        let mut params: Vec<TypedParam> = vec![TypedParam {
                            name: "__self".to_string(),
                            ty: self_ty_erased,
                            mutable: false,
                            span: method.span,
                        }];
                        for p in &method.params {
                            let pty = resolve::resolve_type_expr(&p.ty, &env, &mut errors);
                            env.define(
                                &p.name,
                                Symbol::Var {
                                    ty: pty.clone(),
                                    mutable: p.mutable,
                                    span: p.span,
                                },
                            );
                            let pty_erased = subst_generic_params_unknown(&pty, &gp_names);
                            params.push(TypedParam {
                                name: p.name.clone(),
                                ty: pty_erased,
                                mutable: p.mutable,
                                span: p.span,
                            });
                        }
                        let body = check::check_typed_block(
                            &method.body,
                            &mut env,
                            &registry,
                            &ret,
                            &mut errors,
                        );
                        env.pop_scope();

                        if ret != ty::Ty::Void
                            && !method.body.stmts.is_empty()
                            && !returns::always_returns(&method.body)
                        {
                            errors.push(AnalysisError::MissingReturn {
                                name: format!("{}::{}", s.name, method.name),
                                span: method.span,
                            });
                        }

                        typed_items.push(TypedItem::Function(TypedFnDef {
                            name: format!("{}_{}", s.name, method.name),
                            params,
                            variadic: method.variadic.as_ref().map(|v| v.name.clone()),
                            return_type: ret,
                            body,
                            is_builtin: false,
                            is_inline: method.annotations.iter().any(|a| a.name == "inline"),
                            is_declaration: false,
                            is_entry: false,
                            is_impure: method.annotations.iter().any(|a| a.name == "impure"),
                            span: method.span,
                        }));
                    }

                    if has_struct_generics {
                        env.pop_scope();
                    }
                }
            }

            Item::Enum(e) => {
                conformance::check_enum_conformance(e, &interfaces, &all_impls, &mut errors);
                let has_generics = !e.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &e.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                let variants: Vec<TypedEnumVariant> = e
                    .variants
                    .iter()
                    .map(|v| TypedEnumVariant {
                        name: v.name.clone(),
                        fields: v
                            .fields
                            .iter()
                            .map(|f| TypedField {
                                name: f.name.clone(),
                                ty: resolve_type_expr(&f.ty, &env, &mut errors),
                                is_priv: f.is_priv,
                                is_indirect: false,
                                span: f.span,
                            })
                            .collect(),
                        discriminant: v.discriminant,
                        span: v.span,
                    })
                    .collect();
                if has_generics {
                    env.pop_scope();
                }
                typed_items.push(TypedItem::Enum(TypedEnumDef {
                    name: e.name.clone(),
                    variants,
                    span: e.span,
                }));
            }

            Item::ImplBlock(impl_block) => {
                conformance::check_impl_completeness(
                    impl_block,
                    &interfaces,
                    &all_impls,
                    &mut errors,
                );
                if impl_block.kind == ImplKind::Plain {
                    let key_ty = match &impl_block.for_type {
                        TypeExpr::Named { name, generics, .. } => {
                            if generics.is_empty() {
                                name.clone()
                            } else {
                                format!("{}[{}]", name, generics.len())
                            }
                        }
                        _ => String::new(),
                    };
                    let key_iface = match &impl_block.interface {
                        TypeExpr::Named { name, generics, .. } => {
                            if generics.is_empty() {
                                name.clone()
                            } else {
                                // Include a fingerprint of generic args so that
                                // `AddableWith[int]` and `AddableWith[float]` do not
                                // collide as duplicate impls for the same type.
                                let args: Vec<String> = generics
                                    .iter()
                                    .map(|g| match g {
                                        TypeExpr::Named { name: n, .. } => n.clone(),
                                        _ => "_".to_string(),
                                    })
                                    .collect();
                                format!("{}[{}]", name, args.join(","))
                            }
                        }
                        _ => String::new(),
                    };
                    if !key_ty.is_empty() && !key_iface.is_empty() {
                        let key = (key_ty.clone(), key_iface.clone());
                        if plain_impls_seen.contains(&key) {
                            errors.push(error::AnalysisError::DuplicateImpl {
                                ty: key_ty,
                                iface: key_iface,
                                span: impl_block.span,
                            });
                        } else {
                            plain_impls_seen.insert(key);
                        }
                    }
                }
                let type_name = match &impl_block.for_type {
                    TypeExpr::Named { name, .. } => name.clone(),
                    _ => "Unknown".to_string(),
                };
                let interface_name = match &impl_block.interface {
                    TypeExpr::Named { name, .. } => name.clone(),
                    _ => String::new(),
                };

                let scope_params: Vec<String> = if !impl_block.generic_params.is_empty() {
                    impl_block
                        .generic_params
                        .iter()
                        .map(|g| g.name.clone())
                        .collect()
                } else {
                    match &impl_block.for_type {
                        TypeExpr::Named { generics, .. } => generics
                            .iter()
                            .filter_map(|g| {
                                if let TypeExpr::Named { name, .. } = g {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        _ => vec![],
                    }
                };
                let has_impl_generics = !scope_params.is_empty();
                if has_impl_generics {
                    env.push_scope();
                    let dummy_span = impl_block.span;
                    for gname in &scope_params {
                        env.define(
                            gname,
                            Symbol::Type {
                                id: TypeId(0),
                                span: dummy_span,
                            },
                        );
                    }
                    register_projection_pins(&impl_block.generic_params, &mut env, &mut errors);
                }

                // Compute self_ty before pushing the Self scope so that `for_type`
                // resolves against the outer env (it never references Self).
                let self_ty = resolve_type_expr(&impl_block.for_type, &env, &mut errors);

                // Push `Self`, any user-defined self alias, and all associated type names
                // from the interface into scope so hook/method signatures resolve correctly.
                env.push_scope();
                env.define("Self", Symbol::TypeAlias(self_ty.clone()));
                if let Some(alias) = &impl_block.self_alias {
                    env.define(alias, Symbol::TypeAlias(self_ty.clone()));
                }
                if let Some(iface_def) = interfaces.iter().find(|i| i.name == interface_name) {
                    use crate::parser::ast::InterfaceItemKind;
                    for iitem in &iface_def.items {
                        if let InterfaceItemKind::AssocType { name, .. } = &iitem.kind {
                            env.define(
                                name,
                                Symbol::Type {
                                    id: TypeId(0),
                                    span: iitem.span,
                                },
                            );
                        }
                    }
                }

                // Collect the struct's field names/types so they can be placed directly
                // in scope inside each method/hook body (field access without `self.`).
                let struct_fields: Vec<(String, Ty)> = registry
                    .get_struct_fields(&type_name)
                    .map(|fs| fs.to_vec())
                    .unwrap_or_default();

                let mut typed_methods: Vec<TypedFnDef> = Vec::new();
                for method in &impl_block.methods {
                    env.push_scope();
                    env.define(
                        "self",
                        Symbol::Var {
                            ty: self_ty.clone(),
                            mutable: false,
                            span: impl_block.span,
                        },
                    );
                    for (fname, fty) in &struct_fields {
                        env.define(fname, Symbol::StructField { ty: fty.clone() });
                    }
                    let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                    let mut params: Vec<TypedParam> = Vec::new();
                    for p in &method.params {
                        let pty = resolve_type_expr(&p.ty, &env, &mut errors);
                        env.define(
                            &p.name,
                            Symbol::Var {
                                ty: pty.clone(),
                                mutable: p.mutable,
                                span: p.span,
                            },
                        );
                        params.push(TypedParam {
                            name: p.name.clone(),
                            ty: pty,
                            mutable: p.mutable,
                            span: p.span,
                        });
                    }
                    let body = check::check_typed_block(
                        &method.body,
                        &mut env,
                        &registry,
                        &ret,
                        &mut errors,
                    );
                    env.pop_scope();
                    typed_methods.push(TypedFnDef {
                        name: method.name.clone(),
                        params,
                        variadic: method.variadic.as_ref().map(|v| v.name.clone()),
                        return_type: ret,
                        body,
                        is_builtin: false,
                        is_inline: method.annotations.iter().any(|a| a.name == "inline"),
                        is_declaration: false,
                        is_entry: false,
                        is_impure: method.annotations.iter().any(|a| a.name == "impure"),
                        span: method.span,
                    });
                }

                // Collect hook names declared @static in the interface definition so
                // impl blocks don't need to repeat the annotation.
                let iface_static_hooks: std::collections::HashSet<String> = interfaces
                    .iter()
                    .find(|i| i.name == interface_name)
                    .map(|iface_def| {
                        use crate::parser::ast::InterfaceItemKind;
                        iface_def
                            .items
                            .iter()
                            .filter_map(|item| {
                                if let InterfaceItemKind::Hook {
                                    annotations, name, ..
                                } = &item.kind
                                {
                                    if annotations.iter().any(|a| a.name == "static") {
                                        return Some(match name {
                                            crate::parser::ast::HookName::Named(n) => n.clone(),
                                            crate::parser::ast::HookName::Op(s) => s.clone(),
                                        });
                                    }
                                }
                                None
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let mut typed_hooks: Vec<TypedHookDef> = Vec::new();
                for hook in &impl_block.hooks {
                    env.push_scope();
                    env.define(
                        "self",
                        Symbol::Var {
                            ty: self_ty.clone(),
                            mutable: false,
                            span: impl_block.span,
                        },
                    );
                    for (fname, fty) in &struct_fields {
                        env.define(fname, Symbol::StructField { ty: fty.clone() });
                    }
                    let ret = hook
                        .return_type
                        .as_ref()
                        .map(|r| resolve_type_expr(r, &env, &mut errors))
                        .unwrap_or(Ty::Void);
                    let mut params: Vec<TypedParam> = Vec::new();
                    for p in &hook.params {
                        let pty = resolve_type_expr(&p.ty, &env, &mut errors);
                        env.define(
                            &p.name,
                            Symbol::Var {
                                ty: pty.clone(),
                                mutable: p.mutable,
                                span: p.span,
                            },
                        );
                        params.push(TypedParam {
                            name: p.name.clone(),
                            ty: pty,
                            mutable: p.mutable,
                            span: p.span,
                        });
                    }
                    let body = check::check_typed_block(
                        &hook.body,
                        &mut env,
                        &registry,
                        &ret,
                        &mut errors,
                    );
                    env.pop_scope();
                    typed_hooks.push(TypedHookDef {
                        is_static: hook.annotations.iter().any(|a| a.name == "static")
                            || iface_static_hooks.contains(&match &hook.name {
                                crate::parser::ast::HookName::Named(n) => n.clone(),
                                crate::parser::ast::HookName::Op(s) => s.clone(),
                            }),
                        is_impure: hook.annotations.iter().any(|a| a.name == "impure"),
                        name: hook.name.clone(),
                        params,
                        return_type: ret,
                        body,
                        span: hook.span,
                    });
                }

                env.pop_scope(); // assoc-types + Self scope
                if has_impl_generics {
                    env.pop_scope();
                }
                typed_items.push(TypedItem::ImplBlock(TypedImplBlock {
                    interface: interface_name,
                    for_type: type_name,
                    for_type_ty: self_ty,
                    kind: impl_block.kind.clone(),
                    methods: typed_methods,
                    hooks: typed_hooks,
                    span: impl_block.span,
                }));
            }

            Item::Interface(iface) => {
                use crate::parser::ast::InterfaceItemKind;
                env.push_scope();
                let dummy_span = iface.span;
                // `Self` stands for the implementing type inside an interface body.
                env.define(
                    "Self",
                    Symbol::Type {
                        id: TypeId(0),
                        span: dummy_span,
                    },
                );
                // Generic type params declared on the interface (e.g. `interface Add[Rhs]`).
                for gp in &iface.generic_params {
                    env.define(
                        &gp.name,
                        Symbol::Type {
                            id: TypeId(0),
                            span: gp.span,
                        },
                    );
                }
                // Associated types declared in the interface body.
                for iitem in &iface.items {
                    if let InterfaceItemKind::AssocType { name, .. } = &iitem.kind {
                        env.define(
                            name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: iitem.span,
                            },
                        );
                    }
                }
                let mut typed_methods: Vec<TypedInterfaceMethod> = Vec::new();
                for iitem in &iface.items {
                    if let InterfaceItemKind::Method(m) = &iitem.kind {
                        let has_method_generics = !m.generic_params.is_empty();
                        if has_method_generics {
                            env.push_scope();
                            for gp in &m.generic_params {
                                env.define(
                                    &gp.name,
                                    Symbol::Type {
                                        id: TypeId(0),
                                        span: gp.span,
                                    },
                                );
                            }
                        }
                        let params: Vec<TypedParam> = m
                            .params
                            .iter()
                            .map(|p| TypedParam {
                                name: p.name.clone(),
                                ty: resolve_type_expr(&p.ty, &env, &mut errors),
                                mutable: p.mutable,
                                span: p.span,
                            })
                            .collect();
                        let return_type = resolve_type_expr(&m.return_type, &env, &mut errors);
                        if has_method_generics {
                            env.pop_scope();
                        }
                        typed_methods.push(TypedInterfaceMethod {
                            name: m.name.clone(),
                            params,
                            return_type,
                            span: m.span,
                        });
                    }
                }
                env.pop_scope();
                typed_items.push(TypedItem::Interface(TypedInterfaceDef {
                    name: iface.name.clone(),
                    methods: typed_methods,
                    span: iface.span,
                }));
            }

            Item::Global(g) => {
                let ty = resolve_type_expr(&g.ty, &env, &mut errors);
                let init = infer::infer_typed_expr(&g.value, &env, &registry, &mut errors);
                typed_items.push(TypedItem::Global(TypedGlobalVar {
                    name: g.name.clone(),
                    ty,
                    init,
                    mutable: g.mutable,
                    span: g.span,
                }));
            }

            Item::Const(c) => {
                // Symbol was already registered in pass 1a. Produce the typed item.
                if let Some(env::Symbol::Const { ty, value, .. }) = env.lookup(&c.name) {
                    let ty = ty.clone();
                    let value = value.clone();
                    typed_items.push(TypedItem::Const(
                        crate::analyzer::typed_ast::TypedConstDef {
                            name: c.name.clone(),
                            ty,
                            value,
                            span: c.span,
                        },
                    ));
                }
            }

            Item::ProcessorDef(proc) => {
                let param_ty = resolve_type_expr(&proc.target_param.ty, &env, &mut errors);
                let ret_ty = if let Some(ret) = &proc.return_type {
                    resolve_type_expr(ret, &env, &mut errors)
                } else {
                    Ty::Void
                };
                env.push_scope();
                env.define(
                    &proc.target_param.name,
                    Symbol::Var {
                        ty: param_ty,
                        mutable: false,
                        span: proc.target_param.span,
                    },
                );
                check::check_typed_block(&proc.body, &mut env, &registry, &ret_ty, &mut errors);
                env.pop_scope();
            }

            _ => {}
        }
    }

    let typed_file = TypedFile {
        items: typed_items,
        span: source.span,
    };

    // Pass 3: constraint collection + solving (interface bound checks).
    let constraints = constrain::collect_constraints(&typed_file);
    errors.extend(solve::solve(&constraints, &registry));

    // Pass 3a: boolean constraint propagation warnings (tautologies/contradictions).
    errors.extend(crate::analyzer::constraints::check_tautological_conditions(
        &typed_file,
    ));

    // Pass 3b: static division-by-zero detection (after constant folding in analysis).
    errors.extend(crate::analyzer::fold::check_division_by_zero(&typed_file));

    // Pass 4: object safety -- check that interfaces used as dynamic types are object-safe.
    {
        use crate::parser::ast::{HookName, InterfaceItemKind};
        // Build the non-erasable set: iface_name -> violating method/hook name.
        // An interface is not erasable when any hook or method:
        //   - uses Self in a non-receiver parameter type, or
        //   - uses Self in the return type, or
        //   - is itself generic (a method with its own type parameters).
        let non_object_safe: std::collections::HashMap<String, String> = interfaces
            .iter()
            .filter_map(|iface| {
                for iitem in &iface.items {
                    match &iitem.kind {
                        InterfaceItemKind::Method(m) => {
                            if !m.generic_params.is_empty()
                                || type_expr_uses_self(&m.return_type)
                                || m.params.iter().any(|p| type_expr_uses_self(&p.ty))
                            {
                                return Some((iface.name.clone(), m.name.clone()));
                            }
                        }
                        InterfaceItemKind::Hook {
                            name,
                            params,
                            return_type,
                            ..
                        } => {
                            let hook_str = match name {
                                HookName::Named(n) => n.clone(),
                                HookName::Op(op) => op.clone(),
                            };
                            let ret_bad = return_type
                                .as_ref()
                                .map(type_expr_uses_self)
                                .unwrap_or(false);
                            if ret_bad || params.iter().any(|p| type_expr_uses_self(&p.ty)) {
                                return Some((iface.name.clone(), hook_str));
                            }
                        }
                        _ => {}
                    }
                }
                None
            })
            .collect();
        // Scan all type positions in the typed file for uses of non-erasable interfaces.
        for item in &typed_file.items {
            match item {
                TypedItem::Function(f) => {
                    check_params_object_safe(&f.params, &non_object_safe, &mut errors, f.span);
                    check_ty_object_safe(&f.return_type, &non_object_safe, &mut errors, f.span);
                }
                TypedItem::ImplBlock(ib) => {
                    for m in &ib.methods {
                        check_params_object_safe(&m.params, &non_object_safe, &mut errors, m.span);
                        check_ty_object_safe(&m.return_type, &non_object_safe, &mut errors, m.span);
                    }
                    for h in &ib.hooks {
                        check_params_object_safe(&h.params, &non_object_safe, &mut errors, h.span);
                        check_ty_object_safe(&h.return_type, &non_object_safe, &mut errors, h.span);
                    }
                }
                TypedItem::Struct(s) => {
                    for field in &s.fields {
                        check_ty_object_safe(&field.ty, &non_object_safe, &mut errors, s.span);
                    }
                }
                _ => {}
            }
        }
    }

    if errors.is_empty() {
        Ok((typed_file, registry))
    } else {
        Err(errors)
    }
}

fn type_expr_uses_self(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Named { name, generics, .. } => {
            name == "Self" || generics.iter().any(type_expr_uses_self)
        }
        TypeExpr::Projection { base, .. } => base == "Self",
        TypeExpr::Union(vs, _) => vs.iter().any(type_expr_uses_self),
        TypeExpr::Tuple(ts, _) => ts.iter().any(type_expr_uses_self),
        TypeExpr::Callable { params, ret, .. } => {
            params.iter().any(type_expr_uses_self) || type_expr_uses_self(ret)
        }
        TypeExpr::Ref { inner, .. } => type_expr_uses_self(inner),
        TypeExpr::GenSplice(..) => false,
        TypeExpr::Compound(parts, _) => parts.iter().any(type_expr_uses_self),
    }
}

fn warn_deprecated_in_block(
    block: &crate::parser::ast::Block,
    deprecated: &std::collections::HashSet<String>,
    caller: &str,
) {
    for stmt in &block.stmts {
        warn_deprecated_in_stmt(stmt, deprecated, caller);
    }
}

fn warn_deprecated_in_stmt(
    stmt: &crate::parser::ast::Stmt,
    deprecated: &std::collections::HashSet<String>,
    caller: &str,
) {
    use crate::parser::ast::Stmt;
    match stmt {
        Stmt::Expr(e)
        | Stmt::Return { value: Some(e), .. }
        | Stmt::Raise { value: Some(e), .. } => {
            warn_deprecated_in_expr(e, deprecated, caller);
        }
        Stmt::VarDecl { value, .. } | Stmt::Assign { value, .. } => {
            warn_deprecated_in_expr(value, deprecated, caller);
        }
        Stmt::CompoundAssign { rhs, .. } => {
            warn_deprecated_in_expr(rhs, deprecated, caller);
        }
        Stmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                warn_deprecated_in_expr(cond, deprecated, caller);
                warn_deprecated_in_block(body, deprecated, caller);
            }
            if let Some(b) = else_branch {
                warn_deprecated_in_block(b, deprecated, caller);
            }
        }
        Stmt::While { cond, body, .. } => {
            warn_deprecated_in_expr(cond, deprecated, caller);
            warn_deprecated_in_block(body, deprecated, caller);
        }
        Stmt::For { iterable, body, .. } => {
            warn_deprecated_in_expr(iterable, deprecated, caller);
            warn_deprecated_in_block(body, deprecated, caller);
        }
        Stmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            warn_deprecated_in_block(body, deprecated, caller);
            for h in handlers {
                warn_deprecated_in_block(&h.body, deprecated, caller);
            }
            if let Some(b) = finally {
                warn_deprecated_in_block(b, deprecated, caller);
            }
        }
        _ => {}
    }
}

fn warn_deprecated_in_expr(
    expr: &crate::parser::ast::Expr,
    deprecated: &std::collections::HashSet<String>,
    caller: &str,
) {
    use crate::parser::ast::Expr;
    match expr {
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(name, _) = callee.as_ref() {
                if deprecated.contains(name) {
                    eprintln!(
                        "warning: call to deprecated function '{}' in '{}'",
                        name, caller
                    );
                }
            }
            warn_deprecated_in_expr(callee, deprecated, caller);
            for a in args {
                warn_deprecated_in_expr(a, deprecated, caller);
            }
        }
        Expr::BinOp { left, right, .. } => {
            warn_deprecated_in_expr(left, deprecated, caller);
            warn_deprecated_in_expr(right, deprecated, caller);
        }
        Expr::UnOp { operand: inner, .. } => warn_deprecated_in_expr(inner, deprecated, caller),
        Expr::Field { object, .. } => warn_deprecated_in_expr(object, deprecated, caller),
        Expr::Tuple(exprs, _) => {
            for e in exprs {
                warn_deprecated_in_expr(e, deprecated, caller);
            }
        }
        _ => {}
    }
}

fn check_ty_object_safe(
    ty: &ty::Ty,
    non_object_safe: &std::collections::HashMap<String, String>,
    errors: &mut Vec<error::AnalysisError>,
    span: Span,
) {
    if let ty::Ty::Interface(_, ref iface_name) = ty {
        if let Some(method_name) = non_object_safe.get(iface_name) {
            errors.push(error::AnalysisError::NonObjectSafeInterface {
                iface: iface_name.clone(),
                method: method_name.clone(),
                span,
            });
        }
    }
}

fn check_params_object_safe(
    params: &[typed_ast::TypedParam],
    non_object_safe: &std::collections::HashMap<String, String>,
    errors: &mut Vec<error::AnalysisError>,
    span: Span,
) {
    for p in params {
        check_ty_object_safe(&p.ty, non_object_safe, errors, span);
    }
}
