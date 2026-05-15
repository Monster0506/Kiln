use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{TypedBlock, TypedHookDef, TypedItem, TypedParam};
use crate::analyzer::TypedFile;
use crate::codegen::context::CodegenContext;
use crate::codegen::exceptions::declare_exception_runtime;
use crate::codegen::exprs::{LowerCtx, VarEnv};
use crate::codegen::memory::declare_alloc_fns;
use crate::codegen::stmts::{block_needs_term, lower_typed_block, LoopCtx};
use crate::codegen::strings::declare_str_runtime;
use crate::codegen::structs::StructLayouts;
use crate::codegen::types::clif_type;
use crate::parser::ast::HookName;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, FuncOrDataId, Linkage, Module};
use std::collections::{HashMap, HashSet};

fn encode_op(op: &str) -> String {
    match op {
        "+" => "add".into(),
        "-" => "sub".into(),
        "*" => "mul".into(),
        "/" => "div".into(),
        "==" => "eq".into(),
        "<=>" => "cmp".into(),
        "[]" => "index".into(),
        "<" => "lt".into(),
        ">" => "gt".into(),
        "<=" => "lte".into(),
        ">=" => "gte".into(),
        other => format!(
            "op_{}",
            other.chars().map(|c| c as u32).fold(0u32, |a, b| a ^ b)
        ),
    }
}

fn hook_qualified_name(type_name: &str, hook: &TypedHookDef) -> String {
    let suffix = match &hook.name {
        HookName::Named(n) => n.clone(),
        HookName::Op(op) => encode_op(op),
    };
    format!("{}_{}", type_name, suffix)
}

fn hook_method_key(hook: &TypedHookDef) -> String {
    match &hook.name {
        HookName::Named(n) => n.clone(),
        HookName::Op(op) => op.clone(),
    }
}

fn register_fn(
    name: &str,
    has_self: bool,
    params: &[TypedParam],
    return_type: &Ty,
    module: &mut cranelift_object::ObjectModule,
) -> FuncId {
    let mut sig = module.make_signature();
    if has_self {
        sig.params.push(AbiParam::new(types::I64));
    }
    for p in params {
        if let Some(t) = clif_type(&p.ty) {
            sig.params.push(AbiParam::new(t));
        }
    }
    if let Some(ret) = clif_type(return_type) {
        sig.returns.push(AbiParam::new(ret));
    }
    module
        .declare_function(name, Linkage::Export, &sig)
        .unwrap_or_else(|_| match module.get_name(name) {
            Some(FuncOrDataId::Func(id)) => id,
            _ => panic!("failed to declare function '{}'", name),
        })
}

struct FnJob {
    name: String,
    func_id: FuncId,
    params: Vec<(String, Ty)>,
    return_type: Ty,
    body: TypedBlock,
    self_type: Option<String>,
}

/// Compile a typed Kiln source file into `cgx.module`.
pub fn compile(typed_file_in: &TypedFile, cgx: &mut CodegenContext) -> Result<(), String> {
    let mono_file = crate::codegen::mono::monomorphize(typed_file_in.clone());
    let typed_file = &mono_file;
    declare_alloc_fns(&mut cgx.module);
    let runtime_ids = declare_str_runtime(&mut cgx.module);
    declare_exception_runtime(&mut cgx.module);

    {
        let mut sig = cgx.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        cgx.module
            .declare_function("__kiln_spawn", Linkage::Import, &sig)
            .ok();
    }

    // Pass 0: build struct/enum layouts.
    let mut layouts = StructLayouts::new();
    for item in &typed_file.items {
        match item {
            TypedItem::Struct(st) if !st.is_builtin => layouts.register_typed_struct(st),
            TypedItem::Enum(en) => layouts.register_typed_enum(en),
            _ => {}
        }
    }

    // Pass 1: register all function prototypes.
    // Pre-seed func_ids with C runtime imports so Kiln code can call them by name.
    let mut func_ids: HashMap<String, FuncId> = runtime_ids.clone();
    let mut fn_jobs: Vec<FnJob> = Vec::new();

    for item in &typed_file.items {
        match item {
            TypedItem::Function(f) => {
                // If already pre-seeded as a runtime import, skip re-declaration.
                let id = if let Some(&existing) = func_ids.get(&f.name) {
                    existing
                } else {
                    let id =
                        register_fn(&f.name, false, &f.params, &f.return_type, &mut cgx.module);
                    func_ids.insert(f.name.clone(), id);
                    id
                };
                let params = f
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone()))
                    .collect();
                fn_jobs.push(FnJob {
                    name: f.name.clone(),
                    func_id: id,
                    params,
                    return_type: f.return_type.clone(),
                    body: f.body.clone(),
                    self_type: None,
                });
            }

            TypedItem::ImplBlock(impl_block) => {
                let type_name = &impl_block.for_type;
                let type_id = layouts.get_type_id(type_name).unwrap_or(0);

                for method in &impl_block.methods {
                    let qualified = format!("{}_{}", type_name, method.name);
                    let id = register_fn(
                        &qualified,
                        true,
                        &method.params,
                        &method.return_type,
                        &mut cgx.module,
                    );
                    func_ids.insert(method.name.clone(), id);
                    func_ids.insert(qualified.clone(), id);
                    layouts.register_vtable_entry(&method.name, type_id, id);
                    let params = method
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.ty.clone()))
                        .collect();
                    fn_jobs.push(FnJob {
                        name: qualified,
                        func_id: id,
                        params,
                        return_type: method.return_type.clone(),
                        body: method.body.clone(),
                        self_type: Some(type_name.clone()),
                    });
                }

                for hook in &impl_block.hooks {
                    let func_name = hook_qualified_name(type_name, hook);
                    let method_key = hook_method_key(hook);
                    let id = register_fn(
                        &func_name,
                        true,
                        &hook.params,
                        &hook.return_type,
                        &mut cgx.module,
                    );
                    func_ids.insert(func_name.clone(), id);
                    layouts.register_vtable_entry(&method_key, type_id, id);
                    let params = hook
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.ty.clone()))
                        .collect();
                    fn_jobs.push(FnJob {
                        name: func_name,
                        func_id: id,
                        params,
                        return_type: hook.return_type.clone(),
                        body: hook.body.clone(),
                        self_type: Some(type_name.clone()),
                    });
                }
            }

            _ => {}
        }
    }

    // Pass 2: compile each function body.
    let mut fbc = FunctionBuilderContext::new();
    // Pre-seed defined_ids so @builtin wrappers for runtime imports are never compiled.
    let mut defined_ids: HashSet<FuncId> = runtime_ids.values().copied().collect();
    let mut defined_thunks: HashSet<String> = HashSet::new();

    for job in fn_jobs {
        if !defined_ids.insert(job.func_id) {
            continue;
        }
        let mut ctx = cgx.module.make_context();

        if job.self_type.is_some() {
            ctx.func.signature.params.push(AbiParam::new(types::I64));
        }
        for (_, ty) in &job.params {
            if let Some(t) = clif_type(ty) {
                ctx.func.signature.params.push(AbiParam::new(t));
            }
        }

        let is_main = job.name == "main";
        let (has_return_val, return_clif_type) = if is_main {
            ctx.func.signature.returns.push(AbiParam::new(types::I64));
            (true, Some(types::I64))
        } else if let Some(ret_ty) = clif_type(&job.return_type) {
            ctx.func.signature.returns.push(AbiParam::new(ret_ty));
            (true, Some(ret_ty))
        } else {
            (false, None)
        };

        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let mut vars = VarEnv::new();
        let block_params: Vec<_> = builder.block_params(entry).to_vec();
        let mut param_idx = 0usize;

        if job.self_type.is_some() {
            if let Some(&val) = block_params.get(param_idx) {
                let v = vars.declare("__self", types::I64, &mut builder);
                builder.def_var(v, val);
            }
            param_idx += 1;
        }

        for (name, ty) in &job.params {
            if let Some(&val) = block_params.get(param_idx) {
                let clif_ty = clif_type(ty).unwrap_or(types::I64);
                let v = vars.declare(name, clif_ty, &mut builder);
                builder.def_var(v, val);
                if ty == &Ty::Str {
                    vars.mark_str(name);
                }
            }
            param_idx += 1;
        }

        let mut loops: Vec<LoopCtx> = vec![];
        let mut lower_ctx = LowerCtx {
            module: &mut cgx.module,
            layouts: &layouts,
            func_ids: &func_ids,
            closure_counter: 0,
            self_type: job.self_type.clone(),
            return_clif_type,
            defined_thunks: &mut defined_thunks,
        };
        lower_typed_block(
            &job.body,
            &mut builder,
            &mut vars,
            &mut loops,
            &mut lower_ctx,
        );

        if block_needs_term(&builder) {
            if has_return_val {
                let ret_ty = return_clif_type.unwrap_or(types::I64);
                let zero = builder.ins().iconst(ret_ty, 0);
                builder.ins().return_(&[zero]);
            } else {
                builder.ins().return_(&[]);
            }
        }

        builder.finalize();

        cgx.module
            .define_function(job.func_id, &mut ctx)
            .map_err(|e| format!("codegen error in '{}': {:?}", job.name, e))?;
    }

    Ok(())
}
