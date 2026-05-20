use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{
    TypedBlock, TypedClosureBody, TypedExpr, TypedExprKind, TypedStmt, TypedStringSegment,
};
use crate::codegen::match_::lower_typed_match;
use crate::codegen::memory::{emit_malloc, load_field, store_field};
use crate::codegen::strings::emit_str_literal;
use crate::codegen::structs::StructLayouts;
use crate::parser::ast::{BinOp, UnOp};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, InstBuilder, MemFlags, StackSlotData, StackSlotKind, Type, Value,
};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_module::{DataId, FuncId, FuncOrDataId, Linkage, Module};
use cranelift_object::ObjectModule;
use std::collections::{HashMap, HashSet};

use crate::codegen::stmts::LoopCtx;

/// Lowering context threaded through all expression/statement lowering.
pub struct LowerCtx<'a> {
    pub module: &'a mut ObjectModule,
    pub layouts: &'a StructLayouts,
    pub func_ids: &'a HashMap<String, FuncId>,
    pub global_vars: &'a HashMap<String, DataId>,
    pub closure_counter: usize,
    pub self_type: Option<String>,
    pub return_clif_type: Option<Type>,
    pub defined_thunks: &'a mut HashSet<String>,
    /// Bodies of @inline functions available for expansion at call sites.
    #[allow(clippy::type_complexity)]
    pub inline_bodies: &'a HashMap<String, (Vec<(String, Ty)>, TypedBlock)>,
}

/// Substitute Ident nodes in a TypedExpr based on a parameter->argument map.
/// Only handles the expression forms produced by simple arithmetic helpers.
fn inline_subst(expr: TypedExpr, subst: &HashMap<String, TypedExpr>) -> TypedExpr {
    let ty = expr.ty.clone();
    let span = expr.span;
    let kind = match expr.kind {
        TypedExprKind::Ident(ref name) => {
            if let Some(rep) = subst.get(name) {
                return rep.clone();
            }
            expr.kind
        }
        TypedExprKind::BinOp { op, left, right } => TypedExprKind::BinOp {
            op,
            left: Box::new(inline_subst(*left, subst)),
            right: Box::new(inline_subst(*right, subst)),
        },
        TypedExprKind::UnOp { op, operand } => TypedExprKind::UnOp {
            op,
            operand: Box::new(inline_subst(*operand, subst)),
        },
        TypedExprKind::Call {
            callee,
            args,
            fn_name,
            generic_bounds,
            generic_params,
            param_tys,
        } => TypedExprKind::Call {
            callee: Box::new(inline_subst(*callee, subst)),
            args: args.into_iter().map(|a| inline_subst(a, subst)).collect(),
            fn_name,
            generic_bounds,
            generic_params,
            param_tys,
        },
        TypedExprKind::Field { object, field } => TypedExprKind::Field {
            object: Box::new(inline_subst(*object, subst)),
            field,
        },
        other => other,
    };
    TypedExpr { kind, ty, span }
}

/// Maps variable names to their Cranelift `Variable` and declared Cranelift type.
#[derive(Clone)]
pub struct VarEnv {
    vars: HashMap<String, Variable>,
    var_types: HashMap<String, Type>,
    str_vars: HashSet<String>,
}

impl Default for VarEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl VarEnv {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
            var_types: HashMap::new(),
            str_vars: HashSet::new(),
        }
    }

    pub fn declare(&mut self, name: &str, ty: Type, builder: &mut FunctionBuilder) -> Variable {
        let var = builder.declare_var(ty);
        self.vars.insert(name.to_string(), var);
        self.var_types.insert(name.to_string(), ty);
        var
    }

    pub fn get(&self, name: &str) -> Option<Variable> {
        self.vars.get(name).copied()
    }

    pub fn get_type(&self, name: &str) -> Option<Type> {
        self.var_types.get(name).copied()
    }

    pub fn mark_str(&mut self, name: &str) {
        self.str_vars.insert(name.to_string());
    }

    pub fn is_str(&self, name: &str) -> bool {
        self.str_vars.contains(name)
    }
}

/// Lower a `TypedExpr` inside a loop context (passes loops for match/break/continue).
pub fn lower_typed_expr_loops(
    expr: &TypedExpr,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) -> Value {
    match &expr.kind {
        TypedExprKind::Match { scrutinee, arms } => {
            let s = lower_typed_expr(scrutinee, builder, vars, ctx);
            lower_typed_match(s, arms, builder, vars, loops, ctx)
        }
        _ => lower_typed_expr(expr, builder, vars, ctx),
    }
}

/// Lower a `TypedExpr` to a Cranelift `Value`.
pub fn lower_typed_expr(
    expr: &TypedExpr,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    ctx: &mut LowerCtx,
) -> Value {
    let raw = lower_typed_expr_inner(expr, builder, vars, ctx);
    // If the expression is semantically a float but came through an I64 ABI
    // boundary (Vec storage, closure param, indirect call return), reinterpret
    // the bits as F64 so arithmetic always operates on the correct machine type.
    if matches!(expr.ty, Ty::Float) && builder.func.dfg.value_type(raw) == types::I64 {
        builder.ins().bitcast(types::F64, MemFlags::new(), raw)
    } else {
        raw
    }
}

fn lower_typed_expr_inner(
    expr: &TypedExpr,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    ctx: &mut LowerCtx,
) -> Value {
    match &expr.kind {
        TypedExprKind::Int(n) => builder.ins().iconst(types::I64, *n),
        TypedExprKind::Float(f) => builder.ins().f64const(*f),
        TypedExprKind::Bool(b) => builder.ins().iconst(types::I8, *b as i64),

        TypedExprKind::Str(segs) => {
            if segs.is_empty() {
                return emit_str_literal("", ctx.module, builder);
            }
            if let [TypedStringSegment::Text(t)] = segs.as_slice() {
                return emit_str_literal(t, ctx.module, builder);
            }
            let mut acc = lower_str_seg(&segs[0], builder, vars, ctx);
            for seg in &segs[1..] {
                let piece = lower_str_seg(seg, builder, vars, ctx);
                acc = call_str_concat(acc, piece, ctx.module, builder);
            }
            acc
        }

        TypedExprKind::Ident(name) => {
            if name == "self" {
                if let Some(self_var) = vars.get("__self") {
                    return builder.use_var(self_var);
                }
            }
            if let Some(&data_id) = ctx.global_vars.get(name.as_str()) {
                let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                let addr = builder.ins().global_value(types::I64, gv);
                return builder.ins().load(types::I64, MemFlags::new(), addr, 0);
            }
            if let Some(var) = vars.get(name) {
                return builder.use_var(var);
            }
            if let Some(&func_id) = ctx.func_ids.get(name.as_str()) {
                // When used as a Callable value, wrap in a fat-pointer via a thunk that
                // accepts (env_ptr, real_args...) and forwards to the real function.
                if let Ty::Callable(param_tys, _) = &expr.ty {
                    let n_params = param_tys.len();
                    let thunk_name = format!("__fn_thunk_{}", name);

                    let mut thunk_sig = ctx.module.make_signature();
                    thunk_sig.params.push(AbiParam::new(types::I64)); // env_ptr (ignored)
                    for _ in 0..n_params {
                        thunk_sig.params.push(AbiParam::new(types::I64));
                    }
                    thunk_sig.returns.push(AbiParam::new(types::I64));

                    let thunk_id = if ctx.defined_thunks.contains(&thunk_name) {
                        match ctx.module.get_name(&thunk_name) {
                            Some(FuncOrDataId::Func(id)) => id,
                            _ => unreachable!(),
                        }
                    } else {
                        let tid = ctx
                            .module
                            .declare_function(&thunk_name, Linkage::Local, &thunk_sig)
                            .unwrap_or_else(|_| match ctx.module.get_name(&thunk_name) {
                                Some(FuncOrDataId::Func(id)) => id,
                                _ => panic!("thunk declaration failed for {}", thunk_name),
                            });

                        use cranelift_frontend::FunctionBuilderContext;
                        let mut fn_ctx = ctx.module.make_context();
                        fn_ctx.func.signature = thunk_sig;
                        let mut fbc = FunctionBuilderContext::new();
                        let mut fn_builder = FunctionBuilder::new(&mut fn_ctx.func, &mut fbc);
                        let entry = fn_builder.create_block();
                        fn_builder.append_block_params_for_function_params(entry);
                        fn_builder.switch_to_block(entry);
                        fn_builder.seal_block(entry);

                        let block_params = fn_builder.block_params(entry).to_vec();
                        let real_args: Vec<_> = block_params[1..].to_vec();

                        let orig_func_ref =
                            ctx.module.declare_func_in_func(func_id, fn_builder.func);
                        let call = fn_builder.ins().call(orig_func_ref, &real_args);
                        let raw = fn_builder
                            .inst_results(call)
                            .first()
                            .copied()
                            .unwrap_or_else(|| fn_builder.ins().iconst(types::I64, 0));
                        // Thunk always returns I64; widen smaller types (e.g. bool = I8).
                        let ret = {
                            let vty = fn_builder.func.dfg.value_type(raw);
                            if vty == types::I64 {
                                raw
                            } else if vty.is_int() {
                                fn_builder.ins().uextend(types::I64, raw)
                            } else if vty.is_float() {
                                fn_builder.ins().bitcast(types::I64, MemFlags::new(), raw)
                            } else {
                                raw
                            }
                        };
                        fn_builder.ins().return_(&[ret]);
                        fn_builder.finalize();
                        if ctx.module.define_function(tid, &mut fn_ctx).is_ok() {
                            ctx.defined_thunks.insert(thunk_name);
                        }
                        tid
                    };

                    let thunk_ref = ctx.module.declare_func_in_func(thunk_id, builder.func);
                    let thunk_addr = builder.ins().func_addr(types::I64, thunk_ref);
                    let fat_ptr = emit_malloc(16, ctx.module, builder);
                    store_field(thunk_addr, fat_ptr, 0, builder);
                    let zero = builder.ins().iconst(types::I64, 0);
                    store_field(zero, fat_ptr, 8, builder);
                    return fat_ptr;
                }

                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                return builder.ins().func_addr(types::I64, func_ref);
            }
            builder.ins().iconst(types::I64, 0)
        }

        TypedExprKind::Tuple(exprs) => {
            let byte_size = (exprs.len() * 8) as u32;
            let ptr = emit_malloc(byte_size, ctx.module, builder);
            for (i, e) in exprs.iter().enumerate() {
                let val = lower_typed_expr(e, builder, vars, ctx);
                let coerced = coerce_to_i64(val, builder);
                store_field(coerced, ptr, (i * 8) as u32, builder);
            }
            ptr
        }

        TypedExprKind::StructLiteral { ty_name, fields } => {
            if let Some(info) = ctx.layouts.get_struct(ty_name) {
                let size = info.size;
                let ptr = emit_malloc(size, ctx.module, builder);
                let type_id = ctx.layouts.get_type_id(ty_name).unwrap_or(0) as i64;
                let tag = builder.ins().iconst(types::I64, type_id);
                store_field(tag, ptr, 0, builder);
                let field_offsets: Vec<(String, u32, bool)> = fields
                    .iter()
                    .filter_map(|(n, _)| {
                        info.field_offset(n)
                            .map(|off| (n.clone(), off, info.is_indirect(n)))
                    })
                    .collect();
                for (i, (_, expr)) in fields.iter().enumerate() {
                    let val = lower_typed_expr(expr, builder, vars, ctx);
                    let coerced = coerce_to_i64(val, builder);
                    if let Some((_, offset, is_indirect)) = field_offsets.get(i) {
                        if *is_indirect {
                            // Heap-allocate 8 bytes for the value, store pointer in struct slot.
                            let field_ptr = emit_malloc(8, ctx.module, builder);
                            builder.ins().store(MemFlags::new(), coerced, field_ptr, 0);
                            store_field(field_ptr, ptr, *offset, builder);
                        } else {
                            store_field(coerced, ptr, *offset, builder);
                        }
                    }
                }
                ptr
            } else if let Some((enum_info, variant_layout)) = ctx.layouts.get_enum_variant(ty_name)
            {
                // Fielded enum variant construction: allocate, write discriminant, write fields.
                let size = enum_info.payload_offset + enum_info.max_payload_size;
                let size = size.div_ceil(8) * 8;
                let ptr = emit_malloc(size.max(8), ctx.module, builder);
                let disc = builder
                    .ins()
                    .iconst(types::I64, variant_layout.discriminant as i64);
                store_field(disc, ptr, 0, builder);
                for (field_name, expr) in fields {
                    if let Some(offset) = variant_layout
                        .fields
                        .iter()
                        .find(|(fn_, _)| fn_ == field_name)
                        .map(|(_, off)| *off)
                    {
                        let val = lower_typed_expr(expr, builder, vars, ctx);
                        let coerced = coerce_to_i64(val, builder);
                        store_field(coerced, ptr, offset, builder);
                    }
                }
                ptr
            } else {
                builder.ins().iconst(types::I64, 0)
            }
        }

        TypedExprKind::Call { callee, args, .. } => {
            // Check for @inline expansion: single-return bodies expand at call site.
            if let TypedExprKind::Ident(fn_name) = &callee.kind {
                if let Some((params, body)) = ctx.inline_bodies.get(fn_name.as_str()) {
                    if body.stmts.len() == 1 {
                        if let TypedStmt::Return {
                            value: Some(ret_expr),
                            ..
                        } = &body.stmts[0]
                        {
                            let subst: HashMap<String, TypedExpr> = params
                                .iter()
                                .zip(args.iter())
                                .map(|((pname, _), arg)| (pname.clone(), arg.clone()))
                                .collect();
                            let expanded = inline_subst(ret_expr.clone(), &subst);
                            eprintln!("[kiln] inlining '{}' at call site", fn_name);
                            return lower_typed_expr(&expanded, builder, vars, ctx);
                        }
                    }
                }
            }
            let arg_vals: Vec<Value> = args
                .iter()
                .map(|a| lower_typed_expr(a, builder, vars, ctx))
                .collect();
            match &callee.kind {
                TypedExprKind::Ident(name) => {
                    // println/print expect a KilnStr pointer. When the argument type is
                    // not Str, call to_str on it first so structs and other types display
                    // correctly instead of being treated as raw KilnStr pointers.
                    if (name == "println" || name == "print")
                        && args.len() == 1
                        && args[0].ty != Ty::Str
                    {
                        let raw = arg_vals[0];
                        let to_str_fn = type_name_of(&args[0].ty)
                            .map(|n| format!("{}_to_str", n))
                            .filter(|fn_name| ctx.func_ids.contains_key(fn_name.as_str()))
                            .unwrap_or_else(|| "__kiln_to_str_dispatch".to_string());
                        let str_val = call_fn_by_name(&to_str_fn, &[raw], builder, ctx);
                        // Call the C runtime directly so we don't re-enter the compiled
                        // generic println body (which would call to_str on the str again).
                        let rt_fn = if name == "println" {
                            "__kiln_println"
                        } else {
                            "__kiln_print"
                        };
                        return call_fn_by_name(rt_fn, &[str_val], builder, ctx);
                    }
                    call_fn_by_name(name, &arg_vals, builder, ctx)
                }
                _ => {
                    let fat_ptr = lower_typed_expr(callee, builder, vars, ctx);
                    indirect_call(fat_ptr, &arg_vals, builder, ctx)
                }
            }
        }

        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            let obj = lower_typed_expr(object, builder, vars, ctx);
            let extra_args: Vec<Value> = args
                .iter()
                .map(|a| lower_typed_expr(a, builder, vars, ctx))
                .collect();
            if matches!(object.ty, crate::analyzer::ty::Ty::Interface(_, _)) {
                let impls: Vec<(u32, FuncId)> =
                    ctx.layouts.all_impls_for_method(method_fn).to_vec();
                lower_vtable_dispatch(obj, method_fn, &extra_args, &impls, builder, ctx)
            } else {
                let mut arg_vals = vec![obj];
                arg_vals.extend(extra_args);
                call_fn_by_name(method_fn, &arg_vals, builder, ctx)
            }
        }

        TypedExprKind::StaticCall { method_fn, args } => {
            let arg_vals: Vec<Value> = args
                .iter()
                .map(|a| lower_typed_expr(a, builder, vars, ctx))
                .collect();
            call_fn_by_name(method_fn, &arg_vals, builder, ctx)
        }

        TypedExprKind::IndirectCall { fat_ptr, args } => {
            let fat_ptr_val = lower_typed_expr(fat_ptr, builder, vars, ctx);
            let arg_vals: Vec<Value> = args
                .iter()
                .map(|a| lower_typed_expr(a, builder, vars, ctx))
                .collect();
            indirect_call(fat_ptr_val, &arg_vals, builder, ctx)
        }

        TypedExprKind::Field { object, field } => {
            let ptr = lower_typed_expr(object, builder, vars, ctx);
            let type_name = type_name_of(&object.ty);
            let offset = if let Some(tn) = &type_name {
                ctx.layouts
                    .field_offset_for_type(tn, field)
                    .or_else(|| ctx.layouts.find_field_offset(field))
            } else {
                ctx.layouts.find_field_offset(field)
            };
            if let Some(off) = offset {
                let field_val = load_field(ptr, off, builder);
                // @indirect fields store a heap pointer; dereference it to get the value.
                if let Some(tn) = &type_name {
                    if ctx.layouts.is_indirect_field(tn, field) {
                        return builder
                            .ins()
                            .load(types::I64, MemFlags::new(), field_val, 0);
                    }
                }
                field_val
            } else {
                builder.ins().iconst(types::I64, 0)
            }
        }

        TypedExprKind::Index { object, index } => {
            let ptr = lower_typed_expr(object, builder, vars, ctx);
            let idx = lower_typed_expr(index, builder, vars, ctx);
            let is_vec = matches!(&object.ty, Ty::Named(_, name, _) if name == "Vec");
            if is_vec {
                call_fn_by_name("Vec_get", &[ptr, idx], builder, ctx)
            } else {
                let offset = builder.ins().imul_imm(idx, 8);
                let addr = builder.ins().iadd(ptr, offset);
                builder.ins().load(types::I64, MemFlags::new(), addr, 0)
            }
        }

        TypedExprKind::BinOp { op, left, right } => {
            let lv = lower_typed_expr(left, builder, vars, ctx);
            let rv = lower_typed_expr(right, builder, vars, ctx);
            // int, float, bool use native LLVM arithmetic; all other types go
            // through their registered hook (str, Named, Vec, etc.).
            let use_native = matches!(left.ty, Ty::Int | Ty::Float | Ty::Bool);
            if !use_native {
                if let Some(type_name) = type_name_of(&left.ty) {
                    if let Some(suffix) = crate::codegen::names::binop_fn_suffix(op) {
                        let fn_name = format!("{}_{}", type_name, suffix);
                        return call_fn_by_name(&fn_name, &[lv, rv], builder, ctx);
                    }
                }
            }
            let (lv, rv) = coerce_binop_operands(lv, rv, builder);
            lower_binop(op, lv, rv, builder, ctx.module)
        }

        TypedExprKind::UnOp { op, operand } => {
            let v = lower_typed_expr(operand, builder, vars, ctx);
            let use_native = matches!(operand.ty, Ty::Int | Ty::Float | Ty::Bool);
            if !use_native {
                if let Some(type_name) = type_name_of(&operand.ty) {
                    let suffix = crate::codegen::names::unop_fn_suffix(op);
                    let fn_name = format!("{}_{}", type_name, suffix);
                    return call_fn_by_name(&fn_name, &[v], builder, ctx);
                }
            }
            let ty = builder.func.dfg.value_type(v);
            match op {
                UnOp::Neg => {
                    if ty.is_float() {
                        builder.ins().fneg(v)
                    } else {
                        builder.ins().ineg(v)
                    }
                }
                UnOp::Not => {
                    let one = builder.ins().iconst(ty, 1);
                    builder.ins().bxor(v, one)
                }
                UnOp::Pos => v,
            }
        }

        TypedExprKind::Unwrap(inner) => lower_typed_expr(inner, builder, vars, ctx),

        TypedExprKind::As {
            expr,
            ty: target_ty,
        } => {
            let v = lower_typed_expr(expr, builder, vars, ctx);
            let src = &expr.ty;
            match (src, target_ty) {
                (s, t) if s == t => v,
                (_, Ty::Str) => {
                    let fn_name = type_name_of(src)
                        .map(|n| format!("{}_to_str", n))
                        .unwrap_or_else(|| "__kiln_to_str_dispatch".to_string());
                    call_fn_by_name(&fn_name, &[v], builder, ctx)
                }
                (Ty::Int, Ty::Float) => builder.ins().fcvt_from_sint(types::F64, v),
                (Ty::Float, Ty::Int) => builder.ins().fcvt_to_sint_sat(types::I64, v),
                _ => v,
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            let s = lower_typed_expr(scrutinee, builder, vars, ctx);
            let mut loops = vec![];
            lower_typed_match(s, arms, builder, vars, &mut loops, ctx)
        }

        TypedExprKind::Closure { params, body } => {
            let name = format!("__kiln_closure_{}", ctx.closure_counter);
            ctx.closure_counter += 1;

            let mut sig = ctx.module.make_signature();
            sig.params.push(AbiParam::new(types::I64)); // env_ptr
            for _ in params {
                sig.params.push(AbiParam::new(types::I64));
            }
            sig.returns.push(AbiParam::new(types::I64));

            let func_id = ctx
                .module
                .declare_function(&name, Linkage::Local, &sig)
                .unwrap_or_else(|_| {
                    if let Some(FuncOrDataId::Func(id)) = ctx.module.get_name(&name) {
                        id
                    } else {
                        panic!("closure declaration failed for {}", name)
                    }
                });

            {
                use crate::codegen::stmts::lower_typed_block;
                use cranelift_frontend::FunctionBuilderContext;

                let mut fn_ctx = ctx.module.make_context();
                fn_ctx.func.signature = sig.clone();
                let mut fbc = FunctionBuilderContext::new();
                let mut fn_builder = FunctionBuilder::new(&mut fn_ctx.func, &mut fbc);
                let entry = fn_builder.create_block();
                fn_builder.append_block_params_for_function_params(entry);
                fn_builder.switch_to_block(entry);
                fn_builder.seal_block(entry);

                let mut inner_vars = VarEnv::new();
                let block_params: Vec<_> = fn_builder.block_params(entry).to_vec();
                for (i, param) in params.iter().enumerate() {
                    if let Some(&val) = block_params.get(i + 1) {
                        let v = inner_vars.declare(&param.name, types::I64, &mut fn_builder);
                        fn_builder.def_var(v, val);
                        if param.ty == Ty::Str {
                            inner_vars.mark_str(&param.name);
                        }
                    }
                }

                let mut inner_loops: Vec<LoopCtx> = vec![];
                let mut inner_ctx = LowerCtx {
                    module: ctx.module,
                    layouts: ctx.layouts,
                    func_ids: ctx.func_ids,
                    global_vars: ctx.global_vars,
                    closure_counter: ctx.closure_counter,
                    self_type: None,
                    return_clif_type: None,
                    defined_thunks: ctx.defined_thunks,
                    inline_bodies: ctx.inline_bodies,
                };

                let result = match body {
                    TypedClosureBody::Expr(e) => Some(lower_typed_expr(
                        e,
                        &mut fn_builder,
                        &mut inner_vars,
                        &mut inner_ctx,
                    )),
                    TypedClosureBody::Block(b) => {
                        lower_typed_block(
                            b,
                            &mut fn_builder,
                            &mut inner_vars,
                            &mut inner_loops,
                            &mut inner_ctx,
                        );
                        None
                    }
                };

                ctx.closure_counter = inner_ctx.closure_counter;

                let needs_term = fn_builder.current_block().is_some_and(|bb| {
                    match fn_builder.func.layout.last_inst(bb) {
                        None => true,
                        Some(inst) => !fn_builder.func.dfg.insts[inst].opcode().is_terminator(),
                    }
                });
                if needs_term {
                    let v = result
                        .map(|r| coerce_to_i64(r, &mut fn_builder))
                        .unwrap_or_else(|| fn_builder.ins().iconst(types::I64, 0));
                    fn_builder.ins().return_(&[v]);
                }
                fn_builder.finalize();
                ctx.module.define_function(func_id, &mut fn_ctx).ok();
            }

            let fat_ptr = emit_malloc(16, ctx.module, builder);
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let fn_addr = builder.ins().func_addr(types::I64, func_ref);
            store_field(fn_addr, fat_ptr, 0, builder);
            let null = builder.ins().iconst(types::I64, 0);
            store_field(null, fat_ptr, 8, builder);
            fat_ptr
        }

        TypedExprKind::Spawn(inner) => {
            let fat_ptr = lower_typed_expr(inner, builder, vars, ctx);
            let fn_ptr = builder.ins().load(types::I64, MemFlags::new(), fat_ptr, 0);
            let env_ptr = builder.ins().load(types::I64, MemFlags::new(), fat_ptr, 8);
            call_spawn(fn_ptr, env_ptr, ctx.module, builder)
        }

        TypedExprKind::Ref { expr, .. } => {
            let val = lower_typed_expr(expr, builder, vars, ctx);
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                8,
                0,
            ));
            builder.ins().stack_store(val, slot, 0);
            builder.ins().stack_addr(types::I64, slot, 0)
        }

        TypedExprKind::Gen { body } => {
            let result_var = builder.declare_var(types::I64);
            let zero = builder.ins().iconst(types::I64, 0);
            builder.def_var(result_var, zero);
            let mut inner_loops: Vec<LoopCtx> = vec![];
            use crate::codegen::stmts::lower_typed_stmt_pub;
            for stmt in &body.stmts {
                if let TypedStmtKind::Expr(e) = lower_typed_stmt_kind(stmt) {
                    let val = lower_typed_expr(e, builder, vars, ctx);
                    let coerced = coerce_to_i64(val, builder);
                    builder.def_var(result_var, coerced);
                } else {
                    lower_typed_stmt_pub(stmt, builder, vars, &mut inner_loops, ctx);
                }
            }
            builder.use_var(result_var)
        }

        TypedExprKind::Array(elems) => {
            let vec_ptr = call_fn_by_name("Vec_new", &[], builder, ctx);
            for elem in elems {
                let val = lower_typed_expr(elem, builder, vars, ctx);
                call_fn_by_name("Vec_add", &[vec_ptr, val], builder, ctx);
            }
            vec_ptr
        }

        TypedExprKind::GenSplice(e) => lower_typed_expr(e, builder, vars, ctx),

        TypedExprKind::EnumVariant { discriminant, .. } => {
            builder.ins().iconst(types::I64, *discriminant)
        }
    }
}

// ---------------------------------------------------------------------------
// Gen block helpers
// ---------------------------------------------------------------------------

enum TypedStmtKind<'a> {
    Expr(&'a TypedExpr),
    Other,
}

fn lower_typed_stmt_kind(stmt: &crate::analyzer::typed_ast::TypedStmt) -> TypedStmtKind<'_> {
    use crate::analyzer::typed_ast::TypedStmt;
    match stmt {
        TypedStmt::Expr(e) => TypedStmtKind::Expr(e),
        _ => TypedStmtKind::Other,
    }
}

// ---------------------------------------------------------------------------
// Call helpers
// ---------------------------------------------------------------------------

fn indirect_call(
    fat_ptr: Value,
    args: &[Value],
    builder: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Value {
    let fn_ptr = builder.ins().load(types::I64, MemFlags::new(), fat_ptr, 0);
    let env_ptr = builder.ins().load(types::I64, MemFlags::new(), fat_ptr, 8);
    let mut sig = ctx.module.make_signature();
    sig.params.push(AbiParam::new(types::I64)); // env
    for _ in args {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    let sig_ref = builder.import_signature(sig);
    let mut call_args = vec![env_ptr];
    for &a in args {
        call_args.push(coerce_to_i64(a, builder));
    }
    let call = builder.ins().call_indirect(sig_ref, fn_ptr, &call_args);
    builder
        .inst_results(call)
        .first()
        .copied()
        .unwrap_or_else(|| builder.ins().iconst(types::I64, 0))
}

/// Call a function by name. If not in func_ids, declare as import with an all-I64 signature.
pub fn call_fn_by_name(
    name: &str,
    args: &[Value],
    builder: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Value {
    let func_id = if let Some(&id) = ctx.func_ids.get(name) {
        id
    } else {
        // Generic method dispatch: when the resolved name is like `T_to_str` (a type
        // parameter + method) and we have no concrete implementation, redirect to the
        // runtime dispatch helper.  This covers generic hook bodies where the receiver
        // type is unknown at compile time (e.g. `item.to_str()` where item: T).
        let dispatch_name: Option<&'static str> = if name.ends_with("_to_str") {
            Some("__kiln_to_str_dispatch")
        } else {
            None
        };
        if let Some(dispatch) = dispatch_name {
            let val = args
                .first()
                .copied()
                .unwrap_or_else(|| builder.ins().iconst(types::I64, 0));
            return call_dispatch(dispatch, val, ctx.module, builder);
        }

        let mut sig = ctx.module.make_signature();
        for _ in args {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        match ctx.module.declare_function(name, Linkage::Import, &sig) {
            Ok(id) => id,
            Err(_) => match ctx.module.get_name(name) {
                Some(FuncOrDataId::Func(id)) => id,
                _ => return builder.ins().iconst(types::I64, 0),
            },
        }
    };
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    let coerced = coerce_for_call(args, func_ref, builder);
    let call = builder.ins().call(func_ref, &coerced);
    builder
        .inst_results(call)
        .first()
        .copied()
        .unwrap_or_else(|| builder.ins().iconst(types::I64, 0))
}

fn coerce_for_call(
    args: &[Value],
    func_ref: cranelift_codegen::ir::FuncRef,
    builder: &mut FunctionBuilder,
) -> Vec<Value> {
    let sig_ref = builder.func.dfg.ext_funcs[func_ref].signature;
    let param_types: Vec<_> = builder.func.dfg.signatures[sig_ref]
        .params
        .iter()
        .map(|p| p.value_type)
        .collect();
    args.iter()
        .enumerate()
        .map(|(i, &arg)| {
            let arg_ty = builder.func.dfg.value_type(arg);
            let expected = param_types.get(i).copied().unwrap_or(arg_ty);
            coerce_value(arg, expected, builder)
        })
        .collect()
}

fn coerce_value(val: Value, target: Type, builder: &mut FunctionBuilder) -> Value {
    let src = builder.func.dfg.value_type(val);
    if src == target {
        return val;
    }
    match (src, target) {
        (types::I8, types::I64) | (types::I32, types::I64) => builder.ins().uextend(target, val),
        (types::I64, types::I8) | (types::I64, types::I32) => builder.ins().ireduce(target, val),
        (types::F64, types::I64) => builder.ins().bitcast(types::I64, MemFlags::new(), val),
        (types::I64, types::F64) => builder.ins().bitcast(types::F64, MemFlags::new(), val),
        _ => val,
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

fn lower_str_seg(
    seg: &TypedStringSegment,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    ctx: &mut LowerCtx,
) -> Value {
    match seg {
        TypedStringSegment::Text(t) => emit_str_literal(t, ctx.module, builder),
        TypedStringSegment::Interp(e) => {
            let v = lower_typed_expr(e, builder, vars, ctx);
            if e.ty == Ty::Str {
                return v;
            }
            // Dispatch to {typename}_to_str hook (e.g. int_to_str, float_to_str).
            // call_fn_by_name already falls back to __kiln_to_str_dispatch for
            // any *_to_str name that isn't registered, so this handles all types.
            let fn_name = type_name_of(&e.ty)
                .map(|n| format!("{}_to_str", n))
                .unwrap_or_else(|| "__kiln_to_str_dispatch".to_string());
            call_fn_by_name(&fn_name, &[v], builder, ctx)
        }
    }
}

/// Call a single-argument runtime dispatch function (e.g. `__kiln_to_str_dispatch`).
fn call_dispatch(
    name: &str,
    val: Value,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) -> Value {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let id = module
        .declare_function(name, Linkage::Import, &sig)
        .unwrap_or_else(|_| {
            if let Some(FuncOrDataId::Func(id)) = module.get_name(name) {
                id
            } else {
                panic!("{name} not found")
            }
        });
    let func_ref = module.declare_func_in_func(id, builder.func);
    let coerced = coerce_to_i64(val, builder);
    let call = builder.ins().call(func_ref, &[coerced]);
    builder.inst_results(call)[0]
}

fn call_str_concat(
    a: Value,
    b: Value,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) -> Value {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let id = module
        .declare_function("__kiln_str_concat", Linkage::Import, &sig)
        .unwrap_or_else(|_| {
            if let Some(FuncOrDataId::Func(id)) = module.get_name("__kiln_str_concat") {
                id
            } else {
                panic!("__kiln_str_concat not found")
            }
        });
    let func_ref = module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(func_ref, &[a, b]);
    builder.inst_results(call)[0]
}

fn call_spawn(
    fn_ptr: Value,
    env_ptr: Value,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) -> Value {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let id = module
        .declare_function("__kiln_spawn", Linkage::Import, &sig)
        .unwrap_or_else(|_| {
            if let Some(FuncOrDataId::Func(id)) = module.get_name("__kiln_spawn") {
                id
            } else {
                panic!("__kiln_spawn not found")
            }
        });
    let func_ref = module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(func_ref, &[fn_ptr, env_ptr]);
    builder.inst_results(call)[0]
}

fn emit_fmod(
    lv: Value,
    rv: Value,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) -> Value {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::F64));
    sig.params.push(AbiParam::new(types::F64));
    sig.returns.push(AbiParam::new(types::F64));
    let id = module
        .declare_function("fmod", Linkage::Import, &sig)
        .unwrap_or_else(|_| {
            if let Some(FuncOrDataId::Func(id)) = module.get_name("fmod") {
                id
            } else {
                panic!("fmod not found")
            }
        });
    let func_ref = module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(func_ref, &[lv, rv]);
    builder.inst_results(call)[0]
}

// ---------------------------------------------------------------------------
// Coercion helpers (public for stmts/match)
// ---------------------------------------------------------------------------

pub fn coerce_to_i64(val: Value, builder: &mut FunctionBuilder) -> Value {
    let ty = builder.func.dfg.value_type(val);
    match ty {
        t if t == types::I64 => val,
        t if t == types::I8 || t == types::I32 => builder.ins().uextend(types::I64, val),
        t if t == types::F64 => builder.ins().bitcast(types::I64, MemFlags::new(), val),
        _ => val,
    }
}

pub fn coerce_to(val: Value, target: Type, builder: &mut FunctionBuilder) -> Value {
    let src = builder.func.dfg.value_type(val);
    if src == target {
        return val;
    }
    match (src, target) {
        (types::I8, types::I64) | (types::I32, types::I64) => {
            builder.ins().uextend(types::I64, val)
        }
        (types::I64, types::I8) => builder.ins().ireduce(types::I8, val),
        (types::I64, types::F64) => builder.ins().fcvt_from_sint(types::F64, val),
        (types::F64, types::I64) => builder.ins().bitcast(types::I64, MemFlags::new(), val),
        (types::I8, types::F64) => {
            let ext = builder.ins().uextend(types::I64, val);
            builder.ins().fcvt_from_sint(types::F64, ext)
        }
        _ => val,
    }
}

fn coerce_binop_operands(lv: Value, rv: Value, builder: &mut FunctionBuilder) -> (Value, Value) {
    let lt = builder.func.dfg.value_type(lv);
    let rt = builder.func.dfg.value_type(rv);
    if lt == types::F64 && rt != types::F64 {
        return (lv, coerce_to(rv, types::F64, builder));
    }
    if rt == types::F64 && lt != types::F64 {
        return (coerce_to(lv, types::F64, builder), rv);
    }
    if lt == types::I8 || rt == types::I8 {
        let lv = if lt == types::I8 {
            builder.ins().uextend(types::I64, lv)
        } else {
            lv
        };
        let rv = if rt == types::I8 {
            builder.ins().uextend(types::I64, rv)
        } else {
            rv
        };
        return (lv, rv);
    }
    (lv, rv)
}

// ---------------------------------------------------------------------------
// Binop lowering
// ---------------------------------------------------------------------------

pub fn lower_binop(
    op: &BinOp,
    lv: Value,
    rv: Value,
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
) -> Value {
    let ty = builder.func.dfg.value_type(lv);
    let is_float = ty.is_float();
    match op {
        BinOp::Add => {
            if is_float {
                builder.ins().fadd(lv, rv)
            } else {
                builder.ins().iadd(lv, rv)
            }
        }
        BinOp::Sub => {
            if is_float {
                builder.ins().fsub(lv, rv)
            } else {
                builder.ins().isub(lv, rv)
            }
        }
        BinOp::Mul => {
            if is_float {
                builder.ins().fmul(lv, rv)
            } else {
                builder.ins().imul(lv, rv)
            }
        }
        BinOp::Div => {
            if is_float {
                builder.ins().fdiv(lv, rv)
            } else {
                builder.ins().sdiv(lv, rv)
            }
        }
        BinOp::Mod => {
            if is_float {
                emit_fmod(lv, rv, module, builder)
            } else {
                builder.ins().srem(lv, rv)
            }
        }
        BinOp::Eq => {
            if is_float {
                builder.ins().fcmp(FloatCC::Equal, lv, rv)
            } else {
                builder.ins().icmp(IntCC::Equal, lv, rv)
            }
        }
        BinOp::Ne => {
            if is_float {
                builder.ins().fcmp(FloatCC::NotEqual, lv, rv)
            } else {
                builder.ins().icmp(IntCC::NotEqual, lv, rv)
            }
        }
        BinOp::Lt => {
            if is_float {
                builder.ins().fcmp(FloatCC::LessThan, lv, rv)
            } else {
                builder.ins().icmp(IntCC::SignedLessThan, lv, rv)
            }
        }
        BinOp::Gt => {
            if is_float {
                builder.ins().fcmp(FloatCC::GreaterThan, lv, rv)
            } else {
                builder.ins().icmp(IntCC::SignedGreaterThan, lv, rv)
            }
        }
        BinOp::LtEq => {
            if is_float {
                builder.ins().fcmp(FloatCC::LessThanOrEqual, lv, rv)
            } else {
                builder.ins().icmp(IntCC::SignedLessThanOrEqual, lv, rv)
            }
        }
        BinOp::GtEq => {
            if is_float {
                builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lv, rv)
            } else {
                builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, lv, rv)
            }
        }
        BinOp::And => builder.ins().band(lv, rv),
        BinOp::Or => builder.ins().bor(lv, rv),
        BinOp::Spaceship => {
            // Returns Ordering discriminants: Less=0, Equal=1, Greater=2
            if is_float {
                let less = builder.ins().iconst(types::I64, 0); // Less
                let equal = builder.ins().iconst(types::I64, 1); // Equal
                let greater = builder.ins().iconst(types::I64, 2); // Greater
                let lt = builder.ins().fcmp(FloatCC::LessThan, lv, rv);
                let gt = builder.ins().fcmp(FloatCC::GreaterThan, lv, rv);
                let gt_val = builder.ins().select(gt, greater, equal);
                builder.ins().select(lt, less, gt_val)
            } else {
                let less = builder.ins().iconst(types::I64, 0); // Less
                let equal = builder.ins().iconst(types::I64, 1); // Equal
                let greater = builder.ins().iconst(types::I64, 2); // Greater
                let lt = builder.ins().icmp(IntCC::SignedLessThan, lv, rv);
                let gt = builder.ins().icmp(IntCC::SignedGreaterThan, lv, rv);
                let gt_val = builder.ins().select(gt, greater, equal);
                builder.ins().select(lt, less, gt_val)
            }
        }
        BinOp::Pipe => lv,
    }
}

// ---------------------------------------------------------------------------
// Vtable dispatch (kept for interface method calls via StructLayouts)
// ---------------------------------------------------------------------------

pub fn lower_vtable_dispatch(
    obj: Value,
    _method_name: &str,
    args: &[Value],
    impls: &[(u32, FuncId)],
    builder: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Value {
    use cranelift_codegen::ir::Block as ClifBlock;
    use cranelift_frontend::Switch;

    let result_var = builder.declare_var(types::I64);
    let zero = builder.ins().iconst(types::I64, 0);
    builder.def_var(result_var, zero);

    let merge_bb = builder.create_block();
    let default_bb = builder.create_block();

    let type_tag = builder.ins().load(types::I64, MemFlags::new(), obj, 0);
    let case_blocks: Vec<(u32, FuncId, ClifBlock)> = impls
        .iter()
        .map(|&(tid, fid)| (tid, fid, builder.create_block()))
        .collect();

    let mut sw = Switch::new();
    for &(type_id, _, bb) in &case_blocks {
        sw.set_entry(type_id as u128, bb);
    }
    sw.emit(builder, type_tag, default_bb);

    for (_, func_id, bb) in case_blocks {
        builder.switch_to_block(bb);
        builder.seal_block(bb);
        let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
        let mut call_args = vec![obj];
        call_args.extend_from_slice(args);
        let coerced = coerce_for_call(&call_args, func_ref, builder);
        let call = builder.ins().call(func_ref, &coerced);
        let result = builder.inst_results(call).first().copied().unwrap_or(zero);
        builder.def_var(result_var, result);
        builder.ins().jump(merge_bb, &[]);
    }

    builder.switch_to_block(default_bb);
    builder.seal_block(default_bb);
    builder.ins().jump(merge_bb, &[]);

    builder.switch_to_block(merge_bb);
    builder.seal_block(merge_bb);
    builder.use_var(result_var)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::typed_ast::{TypedExpr, TypedExprKind};
    use crate::codegen::context::CodegenContext;
    use crate::diagnostics::Span;
    use crate::parser::ast::BinOp;
    use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
    use cranelift_frontend::FunctionBuilderContext;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn mk(kind: TypedExprKind, ty: Ty) -> TypedExpr {
        TypedExpr {
            kind,
            ty,
            span: s(),
        }
    }

    fn with_builder<F: FnOnce(&mut FunctionBuilder, &mut VarEnv, &mut LowerCtx) -> Value>(
        ret_ty: Type,
        f: F,
    ) {
        let mut cgx = CodegenContext::new("test");
        let mut fbc = FunctionBuilderContext::new();
        cgx.ctx.func.signature.returns.push(AbiParam::new(ret_ty));
        let mut builder = FunctionBuilder::new(&mut cgx.ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.switch_to_block(block);
        builder.seal_block(block);
        let mut vars = VarEnv::new();
        let layouts = StructLayouts::new();
        let func_ids = HashMap::new();
        let global_vars = HashMap::new();
        let mut thunks = HashSet::new();
        let empty_inline: HashMap<String, (Vec<(String, Ty)>, TypedBlock)> = HashMap::new();
        let mut lctx = LowerCtx {
            module: &mut cgx.module,
            layouts: &layouts,
            func_ids: &func_ids,
            global_vars: &global_vars,
            closure_counter: 0,
            self_type: None,
            return_clif_type: None,
            defined_thunks: &mut thunks,
            inline_bodies: &empty_inline,
        };
        let val = f(&mut builder, &mut vars, &mut lctx);
        assert_eq!(builder.func.dfg.value_type(val), ret_ty);
        builder.ins().return_(&[val]);
        builder.finalize();
    }

    #[test]
    fn int_literal_emits_i64() {
        with_builder(types::I64, |b, vars, ctx| {
            lower_typed_expr(&mk(TypedExprKind::Int(42), Ty::Int), b, vars, ctx)
        });
    }

    #[test]
    fn bool_true_emits_i8() {
        with_builder(types::I8, |b, vars, ctx| {
            lower_typed_expr(&mk(TypedExprKind::Bool(true), Ty::Bool), b, vars, ctx)
        });
    }

    #[test]
    fn float_literal_emits_f64() {
        with_builder(types::F64, |b, vars, ctx| {
            lower_typed_expr(&mk(TypedExprKind::Float(3.14), Ty::Float), b, vars, ctx)
        });
    }

    #[test]
    fn int_add_emits_i64() {
        with_builder(types::I64, |b, vars, ctx| {
            lower_typed_expr(
                &mk(
                    TypedExprKind::BinOp {
                        op: BinOp::Add,
                        left: Box::new(mk(TypedExprKind::Int(1), Ty::Int)),
                        right: Box::new(mk(TypedExprKind::Int(2), Ty::Int)),
                    },
                    Ty::Int,
                ),
                b,
                vars,
                ctx,
            )
        });
    }
}
