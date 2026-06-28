use crate::analyzer::typed_ast::{TypedMatchArm, TypedPattern};
use crate::codegen::exprs::{coerce_to_i64, lower_typed_expr, LowerCtx, VarEnv};
use crate::codegen::memory::load_field;
use crate::codegen::stmts::LoopCtx;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_frontend::FunctionBuilder;

/// Lower a `match` expression to Cranelift IR.
pub fn lower_typed_match(
    scrutinee: Value,
    arms: &[TypedMatchArm],
    builder: &mut FunctionBuilder,
    vars: &mut VarEnv,
    _loops: &mut Vec<LoopCtx>,
    ctx: &mut LowerCtx,
) -> Value {
    if arms.is_empty() {
        return builder.ins().iconst(types::I64, 0);
    }

    let result_var = builder.declare_var(types::I64);
    let zero = builder.ins().iconst(types::I64, 0);
    builder.def_var(result_var, zero);

    let merge_bb = builder.create_block();
    let mut exhaustive = false;

    for arm in arms {
        match &arm.pattern {
            TypedPattern::Wildcard(_)
            | TypedPattern::TypeBinding { .. }
            | TypedPattern::InterfaceGuard { .. } => {
                if let TypedPattern::TypeBinding { name, .. }
                | TypedPattern::InterfaceGuard { name, .. } = &arm.pattern
                {
                    let v = vars.declare(name, types::I64, builder);
                    let coerced = coerce_to_i64(scrutinee, builder);
                    builder.def_var(v, coerced);
                }
                let val = lower_typed_expr(&arm.body, builder, vars, ctx);
                let result = coerce_to_i64(val, builder);
                builder.def_var(result_var, result);
                builder.ins().jump(merge_bb, &[]);
                exhaustive = true;
                break;
            }

            TypedPattern::Literal(lit_expr) => {
                let lit_val = lower_typed_expr(lit_expr, builder, vars, ctx);
                let arm_bb = builder.create_block();
                let next_bb = builder.create_block();

                let scrut_ty = builder.func.dfg.value_type(scrutinee);
                let cmp = if scrut_ty.is_float() {
                    builder.ins().fcmp(FloatCC::Equal, scrutinee, lit_val)
                } else {
                    let s = coerce_to_i64(scrutinee, builder);
                    let l = coerce_to_i64(lit_val, builder);
                    builder.ins().icmp(IntCC::Equal, s, l)
                };
                builder.ins().brif(cmp, arm_bb, &[], next_bb, &[]);

                builder.switch_to_block(arm_bb);
                builder.seal_block(arm_bb);
                let val = lower_typed_expr(&arm.body, builder, vars, ctx);
                let result = coerce_to_i64(val, builder);
                builder.def_var(result_var, result);
                builder.ins().jump(merge_bb, &[]);

                builder.switch_to_block(next_bb);
                builder.seal_block(next_bb);
            }

            TypedPattern::Struct {
                variant, fields, ..
            } => {
                if let Some(layout) = ctx.layouts.get_struct(variant) {
                    let bindings: Vec<(String, u32)> = fields
                        .iter()
                        .filter(|(_, var_name)| var_name != "_")
                        .filter_map(|(field_name, var_name)| {
                            layout
                                .field_offset(field_name)
                                .map(|off| (var_name.clone(), off))
                        })
                        .collect();
                    for (var_name, offset) in bindings {
                        let field_val = builder.ins().load(
                            types::I64,
                            MemFlags::new(),
                            scrutinee,
                            offset as i32,
                        );
                        let v = vars.declare(&var_name, types::I64, builder);
                        builder.def_var(v, field_val);
                    }
                    let val = lower_typed_expr(&arm.body, builder, vars, ctx);
                    let result = coerce_to_i64(val, builder);
                    builder.def_var(result_var, result);
                    builder.ins().jump(merge_bb, &[]);
                    exhaustive = true;
                    break;
                } else if let Some((enum_info, variant_layout)) =
                    ctx.layouts.get_enum_variant(variant)
                {
                    let expected = builder
                        .ins()
                        .iconst(types::I64, variant_layout.discriminant as i64);

                    let arm_bb = builder.create_block();
                    let next_bb = builder.create_block();

                    if variant_layout.fields.is_empty() {
                        // Unit variant: scrutinee is a raw I64 discriminant, not a pointer.
                        let s64 = coerce_to_i64(scrutinee, builder);
                        let cmp = builder.ins().icmp(IntCC::Equal, s64, expected);
                        builder.ins().brif(cmp, arm_bb, &[], next_bb, &[]);
                    } else {
                        // Fielded variant: if scrutinee equals any unit discriminant it is
                        // a raw integer, not a pointer, so this arm cannot match.
                        let unit_discs: Vec<i64> = enum_info
                            .variants
                            .values()
                            .filter(|vl| vl.fields.is_empty())
                            .map(|vl| vl.discriminant as i64)
                            .collect();
                        let s64 = coerce_to_i64(scrutinee, builder);
                        for unit_disc in unit_discs {
                            let disc_val = builder.ins().iconst(types::I64, unit_disc);
                            let is_unit = builder.ins().icmp(IntCC::Equal, s64, disc_val);
                            let cont_bb = builder.create_block();
                            builder.ins().brif(is_unit, next_bb, &[], cont_bb, &[]);
                            builder.switch_to_block(cont_bb);
                            builder.seal_block(cont_bb);
                        }
                        // Scrutinee is a pointer; load the discriminant and compare.
                        let disc_raw =
                            builder
                                .ins()
                                .load(types::I32, MemFlags::new(), scrutinee, 0);
                        let disc_64 = builder.ins().uextend(types::I64, disc_raw);
                        let cmp = builder.ins().icmp(IntCC::Equal, disc_64, expected);
                        builder.ins().brif(cmp, arm_bb, &[], next_bb, &[]);
                    };

                    builder.switch_to_block(arm_bb);
                    builder.seal_block(arm_bb);

                    let bindings: Vec<(String, u32)> = fields
                        .iter()
                        .filter_map(|(field_name, var_name)| {
                            variant_layout
                                .fields
                                .iter()
                                .find(|(fn_, _)| fn_ == field_name)
                                .map(|(_, off)| (var_name.clone(), *off))
                        })
                        .collect();
                    let _ = enum_info;
                    for (var_name, offset) in bindings {
                        let field_val = builder.ins().load(
                            types::I64,
                            MemFlags::new(),
                            scrutinee,
                            offset as i32,
                        );
                        let v = vars.declare(&var_name, types::I64, builder);
                        builder.def_var(v, field_val);
                    }
                    let val = lower_typed_expr(&arm.body, builder, vars, ctx);
                    let result = coerce_to_i64(val, builder);
                    builder.def_var(result_var, result);
                    builder.ins().jump(merge_bb, &[]);

                    builder.switch_to_block(next_bb);
                    builder.seal_block(next_bb);
                } else {
                    let val = lower_typed_expr(&arm.body, builder, vars, ctx);
                    let result = coerce_to_i64(val, builder);
                    builder.def_var(result_var, result);
                    builder.ins().jump(merge_bb, &[]);
                    exhaustive = true;
                    break;
                }
            }

            TypedPattern::Tuple(patterns, _) => {
                for (i, pat) in patterns.iter().enumerate() {
                    match pat {
                        TypedPattern::Wildcard(_) => {}
                        TypedPattern::TypeBinding { name, .. }
                        | TypedPattern::InterfaceGuard { name, .. } => {
                            let field_val = load_field(scrutinee, (i * 8) as u32, builder);
                            let v = vars.declare(name, types::I64, builder);
                            builder.def_var(v, field_val);
                        }
                        _ => {
                            let _ = load_field(scrutinee, (i * 8) as u32, builder);
                        }
                    }
                }
                let val = lower_typed_expr(&arm.body, builder, vars, ctx);
                let result = coerce_to_i64(val, builder);
                builder.def_var(result_var, result);
                builder.ins().jump(merge_bb, &[]);
                exhaustive = true;
                break;
            }
        }
    }

    if !exhaustive {
        builder.ins().jump(merge_bb, &[]);
    }

    builder.switch_to_block(merge_bb);
    builder.seal_block(merge_bb);

    builder.use_var(result_var)
}
