pub mod check;
pub mod collect;
pub mod conformance;
pub mod env;
pub mod error;
pub mod exhaustive;
pub mod infer;
pub mod resolve;
pub mod returns;
pub mod ty;

pub use error::AnalysisError;
pub use ty::{Ty, TypeId, TypeRegistry};

use crate::parser::ast::{Item, SourceFile};

fn register_builtins(env: &mut env::Env, registry: &mut ty::TypeRegistry) {
    use env::Symbol;
    use ty::{Ty, TypeKind};
    let s = crate::diagnostics::Span::new(0, 0);

    // Built-in types
    let exc_id = registry.register("Exception".into(), TypeKind::Struct);
    env.define(
        "Exception",
        Symbol::Type {
            id: exc_id,
            span: s,
        },
    );

    // Built-in functions (variadic/generic ones get Unknown params so arity isn't checked)
    let fns: &[(&str, &[Ty], Ty)] = &[
        ("print", &[Ty::Unknown], Ty::Void),
        ("println", &[Ty::Unknown], Ty::Void),
        ("len", &[Ty::Unknown], Ty::Int),
        ("panic", &[Ty::Unknown], Ty::Void),
        ("assert", &[Ty::Unknown], Ty::Void),
        ("clock_ms", &[], Ty::Int),
    ];
    for (name, params, ret) in fns {
        env.define(
            name,
            Symbol::Fn {
                generic_params: vec![],
                params: params
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (format!("_{i}"), t.clone()))
                    .collect(),
                ret: ret.clone(),
                span: s,
            },
        );
    }
}

pub fn analyze(source: &SourceFile) -> Result<(), Vec<AnalysisError>> {
    let mut errors: Vec<AnalysisError> = Vec::new();
    let mut env = env::Env::new();
    let mut registry = ty::TypeRegistry::new();

    env.push_scope(); // global scope for builtins
    register_builtins(&mut env, &mut registry);

    // Pass 1: collect top-level names (enables forward references)
    errors.extend(collect::collect_top_level(source, &mut env, &mut registry));

    // Pass 1b: resolve top-level function signatures now that all types are registered.
    // The collection pass registers functions with empty params; this pass fills them in
    // so that cross-function and recursive calls see the correct arity.
    for item in &source.items {
        if let Item::Function(f) = item {
            let ret = resolve::resolve_type_expr(&f.return_type, &env, &registry, &mut errors);
            let params: Vec<(String, ty::Ty)> = f
                .params
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        resolve::resolve_type_expr(&p.ty, &env, &registry, &mut errors),
                    )
                })
                .collect();
            env.define(
                &f.name,
                env::Symbol::Fn {
                    generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
                    params,
                    ret,
                    span: f.span,
                },
            );
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

    // Pass 2: check each top-level item
    for item in &source.items {
        match item {
            Item::Function(f) => {
                let ret = resolve::resolve_type_expr(&f.return_type, &env, &registry, &mut errors);
                env.push_scope();
                for p in &f.params {
                    let pty = resolve::resolve_type_expr(&p.ty, &env, &registry, &mut errors);
                    env.define(
                        &p.name,
                        env::Symbol::Var {
                            ty: pty,
                            mutable: false,
                            span: p.span,
                        },
                    );
                }
                check::check_block(&f.body, &mut env, &registry, &ret, &mut errors);
                env.pop_scope();
                if ret != ty::Ty::Void && !returns::always_returns(&f.body) {
                    errors.push(AnalysisError::MissingReturn {
                        name: f.name.clone(),
                        span: f.span,
                    });
                }
            }
            Item::Struct(s) => {
                conformance::check_struct_conformance(s, &interfaces, &mut errors);
            }
            Item::Enum(e) => {
                conformance::check_enum_conformance(e, &interfaces, &mut errors);
            }
            _ => {}
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
