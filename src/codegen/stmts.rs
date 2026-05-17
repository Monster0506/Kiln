use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{TypedBlock, TypedExpr, TypedExprKind, TypedStmt};
use crate::codegen::exprs::{
    call_fn_by_name, coerce_to, coerce_to_i64, lower_binop, lower_typed_expr_loops, LowerCtx,
    VarEnv,
};
use crate::codegen::memory::store_field;
use crate::codegen::names::binop_fn_suffix;
use crate::codegen::types::clif_type;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, Block as ClifBlock, InstBuilder, MemFlags};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};

/// Returns true if the current block in `builder` does not yet have a terminator.
pub fn block_needs_term(builder: &FunctionBuilder) -> bool {
    let bb = match builder.current_block() {
        Some(b) => b,
        None => return false,
    };
    match builder.func.layout.last_inst(bb) {
        None => true,
        Some(inst) => !builder.func.dfg.insts[inst].opcode().is_terminator(),
    }
}

/// Stack entry tracking the header and exit blocks for the enclosing loop.
pub struct LoopCtx {
    pub header: ClifBlock,
    pub exit: ClifBlock,
}

pub fn lower_typed_block(
    block: &TypedBlock,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    for stmt in &block.stmts {
        lower_typed_stmt(stmt, builder, vars, loops, ctx);
        if !block_needs_term(builder) {
            break;
        }
    }
}

pub fn lower_typed_stmt_pub(
    stmt: &TypedStmt,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    lower_typed_stmt(stmt, builder, vars, loops, ctx);
}

fn lower_typed_stmt(
    stmt: &TypedStmt,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    match stmt {
        TypedStmt::VarDecl {
            name, ty, value, ..
        } => {
            let clif_ty = clif_type(ty).unwrap_or(types::I64);
            let var = vars.declare(name, clif_ty, builder);
            let raw = lower_typed_expr_loops(value, builder, vars, loops, ctx);
            let val = coerce_to(raw, clif_ty, builder);
            builder.def_var(var, val);
            if ty == &Ty::Str {
                vars.mark_str(name);
            }
        }

        TypedStmt::Assign { target, value, .. } => {
            let val = lower_typed_expr_loops(value, builder, vars, loops, ctx);
            match &target.kind {
                TypedExprKind::Ident(name) => {
                    if let Some(&data_id) = ctx.global_vars.get(name.as_str()) {
                        let gv = ctx.module.declare_data_in_func(data_id, &mut builder.func);
                        let addr = builder.ins().global_value(types::I64, gv);
                        let coerced = coerce_to_i64(val, builder);
                        builder.ins().store(MemFlags::new(), coerced, addr, 0);
                    } else if let Some(var) = vars.get(name) {
                        let coerced = if let Some(var_ty) = vars.get_type(name) {
                            coerce_to(val, var_ty, builder)
                        } else {
                            val
                        };
                        builder.def_var(var, coerced);
                    }
                }
                TypedExprKind::Field { object, field } => {
                    let ptr = lower_typed_expr_loops(object, builder, vars, loops, ctx);
                    if let Some(off) = ctx.layouts.find_field_offset(field) {
                        let coerced = coerce_to_i64(val, builder);
                        store_field(coerced, ptr, off, builder);
                    }
                }
                _ => {
                    lower_typed_expr_loops(target, builder, vars, loops, ctx);
                }
            }
        }

        TypedStmt::CompoundAssign {
            target, op, rhs, ..
        } => {
            let rhs_val = lower_typed_expr_loops(rhs, builder, vars, loops, ctx);
            let use_native = matches!(target.ty, Ty::Int | Ty::Float | Ty::Bool);
            match &target.kind {
                TypedExprKind::Ident(name) => {
                    if let Some(&data_id) = ctx.global_vars.get(name.as_str()) {
                        let gv = ctx.module.declare_data_in_func(data_id, &mut builder.func);
                        let addr = builder.ins().global_value(types::I64, gv);
                        let cur = builder.ins().load(types::I64, MemFlags::new(), addr, 0);
                        let result = lower_binop(op, cur, rhs_val, builder, ctx.module);
                        builder.ins().store(MemFlags::new(), result, addr, 0);
                    } else if let Some(var) = vars.get(name) {
                        let cur = builder.use_var(var);
                        let result = if !use_native {
                            if let Some(type_name) = type_name_of(&target.ty) {
                                if let Some(suffix) = binop_fn_suffix(op) {
                                    let fn_name = format!("{}_{}", type_name, suffix);
                                    call_fn_by_name(&fn_name, &[cur, rhs_val], builder, ctx)
                                } else {
                                    lower_binop(op, cur, rhs_val, builder, ctx.module)
                                }
                            } else {
                                lower_binop(op, cur, rhs_val, builder, ctx.module)
                            }
                        } else {
                            lower_binop(op, cur, rhs_val, builder, ctx.module)
                        };
                        builder.def_var(var, result);
                    }
                }
                TypedExprKind::Field { object, field } => {
                    let ptr = lower_typed_expr_loops(object, builder, vars, loops, ctx);
                    if let Some(off) = ctx.layouts.find_field_offset(field) {
                        let cur = crate::codegen::memory::load_field(ptr, off, builder);
                        let result = if !use_native {
                            if let Some(type_name) = type_name_of(&target.ty) {
                                if let Some(suffix) = binop_fn_suffix(op) {
                                    let fn_name = format!("{}_{}", type_name, suffix);
                                    call_fn_by_name(&fn_name, &[cur, rhs_val], builder, ctx)
                                } else {
                                    lower_binop(op, cur, rhs_val, builder, ctx.module)
                                }
                            } else {
                                lower_binop(op, cur, rhs_val, builder, ctx.module)
                            }
                        } else {
                            lower_binop(op, cur, rhs_val, builder, ctx.module)
                        };
                        let coerced = coerce_to_i64(result, builder);
                        store_field(coerced, ptr, off, builder);
                    }
                }
                _ => {
                    lower_typed_expr_loops(target, builder, vars, loops, ctx);
                }
            }
        }

        TypedStmt::Return { value, .. } => match value {
            Some(v) => {
                let raw = lower_typed_expr_loops(v, builder, vars, loops, ctx);
                let val = if let Some(ret_ty) = ctx.return_clif_type {
                    coerce_to(raw, ret_ty, builder)
                } else {
                    raw
                };
                builder.ins().return_(&[val]);
            }
            None => {
                builder.ins().return_(&[]);
            }
        },

        TypedStmt::Expr(expr) => {
            lower_typed_expr_loops(expr, builder, vars, loops, ctx);
        }

        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            lower_if(branches, else_branch.as_ref(), builder, vars, loops, ctx);
        }

        TypedStmt::While { cond, body, .. } => {
            lower_while(cond, body, builder, vars, loops, ctx);
        }

        TypedStmt::DoWhile { body, cond, .. } => {
            lower_do_while(body, cond, builder, vars, loops, ctx);
        }

        TypedStmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            ..
        } => {
            lower_for(
                binding,
                &binding_ty,
                iterable,
                body,
                builder,
                vars,
                loops,
                ctx,
            );
        }

        TypedStmt::Break(_) => {
            if let Some(lctx) = loops.last() {
                let exit = lctx.exit;
                builder.ins().jump(exit, &[]);
            }
        }

        TypedStmt::Continue(_) => {
            if let Some(lctx) = loops.last() {
                let header = lctx.header;
                builder.ins().jump(header, &[]);
            }
        }

        TypedStmt::Raise { value, .. } => {
            let exc_val = match value {
                Some(v) => lower_typed_expr_loops(v, builder, vars, loops, ctx),
                None => builder.ins().iconst(types::I64, 0),
            };
            crate::codegen::exceptions::emit_raise(exc_val, ctx.module, builder);
        }

        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            let handlers_clone = handlers.clone();
            let finally_clone = finally.clone();
            let mut inner_vars = vars.clone();

            {
                use cranelift_codegen::ir::{StackSlotData, StackSlotKind};

                // Allocate the jmp_buf in THIS (Cranelift) function's stack frame.
                // 8 x 8 bytes = 64 bytes: [rip, rsp, rbx, rbp, r12, r13, r14, r15].
                // The jmp_buf lives at the same address for the duration of the try
                // block, so __kiln_longjmp can safely jump back into this frame.
                let slot = builder.func.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    64,
                    8,
                ));
                let jmpbuf_addr = builder.ins().stack_addr(types::I64, slot, 0);

                // __kiln_exc_push(jmpbuf_addr): register this frame
                let push_sig = {
                    let mut s = ctx.module.make_signature();
                    s.params.push(AbiParam::new(types::I64));
                    s
                };
                let push_id = ctx
                    .module
                    .declare_function("__kiln_exc_push", Linkage::Import, &push_sig)
                    .unwrap_or_else(|_| {
                        if let Some(cranelift_module::FuncOrDataId::Func(id)) =
                            ctx.module.get_name("__kiln_exc_push")
                        {
                            id
                        } else {
                            panic!("__kiln_exc_push not found")
                        }
                    });
                let push_ref = ctx.module.declare_func_in_func(push_id, builder.func);
                builder.ins().call(push_ref, &[jmpbuf_addr]);

                // __kiln_setjmp(jmpbuf_addr) -> i32: saves this frame's context.
                // Returns 0 on initial entry; __kiln_longjmp makes it return 1.
                let setjmp_sig = {
                    let mut s = ctx.module.make_signature();
                    s.params.push(AbiParam::new(types::I64));
                    s.returns.push(AbiParam::new(types::I32));
                    s
                };
                let setjmp_id = ctx
                    .module
                    .declare_function("__kiln_setjmp", Linkage::Import, &setjmp_sig)
                    .unwrap_or_else(|_| {
                        if let Some(cranelift_module::FuncOrDataId::Func(id)) =
                            ctx.module.get_name("__kiln_setjmp")
                        {
                            id
                        } else {
                            panic!("__kiln_setjmp not found")
                        }
                    });
                let setjmp_ref = ctx.module.declare_func_in_func(setjmp_id, builder.func);
                let call = builder.ins().call(setjmp_ref, &[jmpbuf_addr]);
                let frame = builder.inst_results(call)[0];

                // __kiln_exc_pop(): unregisters the frame (called on both paths)
                let pop_sig = ctx.module.make_signature();
                let pop_id = ctx
                    .module
                    .declare_function("__kiln_exc_pop", Linkage::Import, &pop_sig)
                    .unwrap_or_else(|_| {
                        if let Some(cranelift_module::FuncOrDataId::Func(id)) =
                            ctx.module.get_name("__kiln_exc_pop")
                        {
                            id
                        } else {
                            panic!("__kiln_exc_pop not found")
                        }
                    });
                let pop_ref = ctx.module.declare_func_in_func(pop_id, builder.func);

                let izero = builder.ins().iconst(types::I32, 0);
                let is_exc = builder.ins().icmp(IntCC::NotEqual, frame, izero);

                let try_body_bb = builder.create_block();
                let exc_bb = builder.create_block();
                let merge_bb = builder.create_block();

                builder.ins().brif(is_exc, exc_bb, &[], try_body_bb, &[]);

                // --- normal (no exception) path ---
                builder.switch_to_block(try_body_bb);
                builder.seal_block(try_body_bb);
                lower_typed_block(body, builder, &mut inner_vars, loops, ctx);
                if block_needs_term(builder) {
                    builder.ins().call(pop_ref, &[]);
                    builder.ins().jump(merge_bb, &[]);
                }

                // --- exception path (reached via longjmp) ---
                builder.switch_to_block(exc_bb);
                builder.seal_block(exc_bb);
                // Pop the frame first so that any raise inside the handler
                // propagates to an outer try-catch rather than looping.
                builder.ins().call(pop_ref, &[]);
                if let Some(handler) = handlers_clone.first() {
                    let mut hv = vars.clone();
                    if !handler.binding.is_empty() {
                        let exc = call_get_current_exc(ctx.module, builder);
                        let v = hv.declare(&handler.binding, types::I64, builder);
                        builder.def_var(v, exc);
                    }
                    lower_typed_block(&handler.body, builder, &mut hv, loops, ctx);
                }
                if block_needs_term(builder) {
                    builder.ins().jump(merge_bb, &[]);
                }

                builder.switch_to_block(merge_bb);
                builder.seal_block(merge_bb);
            }

            if let Some(fin_block) = &finally_clone {
                lower_typed_block(fin_block, builder, vars, loops, ctx);
            }
        }

        TypedStmt::FnDef(_) => {
            // Nested function definitions are hoisted by compile.rs.
        }
    }
}

fn lower_if(
    branches: &[(TypedExpr, TypedBlock)],
    else_branch: Option<&TypedBlock>,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    let merge_bb = builder.create_block();
    let n = branches.len();

    for (i, (cond, then_block)) in branches.iter().enumerate() {
        let then_bb = builder.create_block();
        let false_bb = if (i + 1 < n) || else_branch.is_some() {
            builder.create_block()
        } else {
            merge_bb
        };

        let cond_val = lower_typed_expr_loops(cond, builder, vars, loops, ctx);
        builder.ins().brif(cond_val, then_bb, &[], false_bb, &[]);

        builder.switch_to_block(then_bb);
        builder.seal_block(then_bb);
        lower_typed_block(then_block, builder, vars, loops, ctx);
        if block_needs_term(builder) {
            builder.ins().jump(merge_bb, &[]);
        }

        builder.switch_to_block(false_bb);
        if false_bb != merge_bb {
            builder.seal_block(false_bb);
        }
    }

    if let Some(eb) = else_branch {
        lower_typed_block(eb, builder, vars, loops, ctx);
        if block_needs_term(builder) {
            builder.ins().jump(merge_bb, &[]);
        }
        builder.switch_to_block(merge_bb);
    }

    builder.seal_block(merge_bb);
}

fn lower_while(
    cond: &TypedExpr,
    body: &TypedBlock,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    let header_bb = builder.create_block();
    let body_bb = builder.create_block();
    let exit_bb = builder.create_block();

    builder.ins().jump(header_bb, &[]);
    builder.switch_to_block(header_bb);

    let cond_val = lower_typed_expr_loops(cond, builder, vars, loops, ctx);
    builder.ins().brif(cond_val, body_bb, &[], exit_bb, &[]);

    builder.switch_to_block(body_bb);
    builder.seal_block(body_bb);

    loops.push(LoopCtx {
        header: header_bb,
        exit: exit_bb,
    });
    lower_typed_block(body, builder, vars, loops, ctx);
    loops.pop();

    if block_needs_term(builder) {
        builder.ins().jump(header_bb, &[]);
    }

    builder.seal_block(header_bb);
    builder.switch_to_block(exit_bb);
    builder.seal_block(exit_bb);
}

fn lower_do_while(
    body: &TypedBlock,
    cond: &TypedExpr,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    let body_bb = builder.create_block();
    let exit_bb = builder.create_block();

    builder.ins().jump(body_bb, &[]);
    builder.switch_to_block(body_bb);

    loops.push(LoopCtx {
        header: body_bb,
        exit: exit_bb,
    });
    lower_typed_block(body, builder, vars, loops, ctx);
    loops.pop();

    if block_needs_term(builder) {
        let cond_val = lower_typed_expr_loops(cond, builder, vars, loops, ctx);
        builder.ins().brif(cond_val, body_bb, &[], exit_bb, &[]);
    }

    builder.seal_block(body_bb);
    builder.switch_to_block(exit_bb);
    builder.seal_block(exit_bb);
}

fn lower_for(
    binding: &str,
    binding_ty: &Ty,
    iterable: &TypedExpr,
    body: &TypedBlock,
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) {
    let coll_val = lower_typed_expr_loops(iterable, builder, vars, loops, ctx);
    let is_vec = match &iterable.ty {
        Ty::Named(_, name, _) => name == "Vec" || name == "Set",
        _ => false,
    };

    // For `for x <- EnumType`, the iterable is the enum type itself used as an
    // expression. The loop iterates over discriminants 0..N where N is the
    // variant count, and each iteration binds the discriminant as the value.
    let enum_variant_count: Option<i64> = match &iterable.ty {
        Ty::Named(_, name, args) if args.is_empty() => ctx
            .layouts
            .get_enum(name)
            .map(|info| info.variants.len() as i64),
        _ => None,
    };

    // Store the collection pointer in a variable so it survives across blocks.
    // For enum-type iterables, coll_val is 0 (unused) but we still declare it.
    let coll_name = format!("__for_coll_{}", binding);
    let coll_var = vars.declare(&coll_name, types::I64, builder);
    builder.def_var(coll_var, coll_val);

    let limit = if is_vec {
        call_fn_by_name("Vec_len", &[coll_val], builder, ctx)
    } else if let Some(count) = enum_variant_count {
        builder.ins().iconst(types::I64, count)
    } else {
        coll_val
    };

    // Internal index variable, separate from the user binding.
    let idx_name = format!("__for_idx_{}", binding);
    let idx_var = vars.declare(&idx_name, types::I64, builder);
    let zero = builder.ins().iconst(types::I64, 0);
    builder.def_var(idx_var, zero);

    // User binding variable — element type drives the Cranelift slot type.
    let bind_clif_ty = clif_type(binding_ty).unwrap_or(types::I64);
    let bind_zero = if bind_clif_ty == types::F64 {
        builder.ins().f64const(0.0)
    } else {
        builder.ins().iconst(bind_clif_ty, 0)
    };
    let bind_var = vars.declare(binding, bind_clif_ty, builder);
    builder.def_var(bind_var, bind_zero);

    // header_bb: condition check
    // body_bb:   element setup + user body
    // incr_bb:   index increment (continue target)
    // exit_bb:   loop exit (break target)
    let header_bb = builder.create_block();
    let body_bb = builder.create_block();
    let incr_bb = builder.create_block();
    let exit_bb = builder.create_block();

    builder.ins().jump(header_bb, &[]);
    builder.switch_to_block(header_bb);

    let i_val = builder.use_var(idx_var);
    let cond = builder.ins().icmp(IntCC::SignedLessThan, i_val, limit);
    builder.ins().brif(cond, body_bb, &[], exit_bb, &[]);

    builder.switch_to_block(body_bb);
    builder.seal_block(body_bb);

    // Retrieve the element for this iteration and store in the binding.
    // Vec_get returns I64 (raw bits); coerce to the binding's actual type.
    // For enum-type iterables, the discriminant IS the index (0..N-1).
    if is_vec {
        let coll = builder.use_var(coll_var);
        let idx_now = builder.use_var(idx_var);
        let raw = call_fn_by_name("Vec_get", &[coll, idx_now], builder, ctx);
        let elem = coerce_to(raw, bind_clif_ty, builder);
        builder.def_var(bind_var, elem);
    } else {
        let idx_now = builder.use_var(idx_var);
        builder.def_var(bind_var, idx_now);
    }

    // `continue` must jump to incr_bb (not header_bb) so the index is
    // incremented before re-checking the condition.
    loops.push(LoopCtx {
        header: incr_bb,
        exit: exit_bb,
    });
    lower_typed_block(body, builder, vars, loops, ctx);
    loops.pop();

    // Body fall-through goes to incr_bb.
    if block_needs_term(builder) {
        builder.ins().jump(incr_bb, &[]);
    }

    // incr_bb: increment the index then re-check.
    // Seal here: all predecessors (body fall-through + any continue jumps) are now known.
    builder.switch_to_block(incr_bb);
    builder.seal_block(incr_bb);
    let cur = builder.use_var(idx_var);
    let one = builder.ins().iconst(types::I64, 1);
    let next = builder.ins().iadd(cur, one);
    builder.def_var(idx_var, next);
    builder.ins().jump(header_bb, &[]);

    builder.seal_block(header_bb);
    builder.switch_to_block(exit_bb);
    builder.seal_block(exit_bb);
}

fn call_get_current_exc(
    module: &mut cranelift_object::ObjectModule,
    builder: &mut FunctionBuilder,
) -> cranelift_codegen::ir::Value {
    let mut sig = module.make_signature();
    sig.returns.push(AbiParam::new(types::I64));
    let id = module
        .declare_function("__kiln_current_exc", Linkage::Import, &sig)
        .unwrap_or_else(|_| {
            if let Some(cranelift_module::FuncOrDataId::Func(id)) =
                module.get_name("__kiln_current_exc")
            {
                id
            } else {
                panic!("__kiln_current_exc not found")
            }
        });
    let func_ref = module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(func_ref, &[]);
    builder.inst_results(call)[0]
}
