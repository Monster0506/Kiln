use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{
    TypedBlock, TypedClosureBody, TypedEnumDef, TypedExpr, TypedExprKind, TypedFile, TypedFnDef,
    TypedGlobalVar, TypedHookDef, TypedImplBlock, TypedInterfaceDef, TypedItem, TypedMatchArm,
    TypedParam, TypedPattern, TypedStmt, TypedStringSegment, TypedStructDef,
};
use crate::parser::ast::{BinOp, HookName, UnOp};
use std::collections::HashSet;

pub fn emit_optimized(file: &TypedFile, user_names: &HashSet<String>) -> String {
    let mut out = String::from("# optimizer output\n\n");

    for item in &file.items {
        if let TypedItem::Global(g) = item {
            if user_names.contains(&g.name) {
                out.push_str(&emit_global(g));
                out.push('\n');
            }
        }
    }

    for item in &file.items {
        let chunk = match item {
            TypedItem::Global(_) => continue,
            TypedItem::Const(c) if user_names.contains(&c.name) => {
                let val = emit_expr_kind(&c.value);
                let ty = emit_ty(&c.ty);
                format!("const {}: {} = {}\n", c.name, ty, val)
            }
            TypedItem::Struct(s) if user_names.contains(&s.name) && !s.is_builtin => emit_struct(s),
            TypedItem::Enum(e) if user_names.contains(&e.name) => emit_enum(e),
            TypedItem::Interface(i) if user_names.contains(&i.name) => emit_interface(i),
            TypedItem::ImplBlock(b) if user_names.contains(&b.for_type) => emit_impl(b),
            TypedItem::Function(f)
                if user_names.contains(&f.name) && !f.is_builtin && !f.is_declaration =>
            {
                emit_fn(f, 0)
            }
            _ => continue,
        };
        out.push_str(&chunk);
        out.push('\n');
    }

    out
}

fn indent_str(n: usize) -> String {
    "    ".repeat(n)
}

fn emit_ty(ty: &Ty) -> String {
    format!("{ty}")
}

fn emit_global(g: &TypedGlobalVar) -> String {
    let mut_kw = if g.mutable { "mut " } else { "" };
    format!(
        "{}{}: {} = {}\n",
        mut_kw,
        g.name,
        emit_ty(&g.ty),
        emit_expr(&g.init)
    )
}

fn emit_struct(s: &TypedStructDef) -> String {
    let mut out = format!("struct {} {{\n", s.name);
    for f in &s.fields {
        out.push_str(&format!("    {}: {}\n", f.name, emit_ty(&f.ty)));
    }
    out.push_str("}\n");
    out
}

fn emit_enum(e: &TypedEnumDef) -> String {
    let mut out = format!("enum {} {{\n", e.name);
    for v in &e.variants {
        if v.fields.is_empty() {
            out.push_str(&format!("    {}\n", v.name));
        } else {
            let fields: Vec<String> = v
                .fields
                .iter()
                .map(|f| format!("{}: {}", f.name, emit_ty(&f.ty)))
                .collect();
            out.push_str(&format!("    {} {{ {} }}\n", v.name, fields.join(", ")));
        }
    }
    out.push_str("}\n");
    out
}

fn emit_interface(i: &TypedInterfaceDef) -> String {
    let mut out = format!("interface {} {{\n", i.name);
    for m in &i.methods {
        let params: Vec<String> = m.params.iter().map(emit_param).collect();
        out.push_str(&format!(
            "    def {}({}) -> {} {{}}\n",
            m.name,
            params.join(", "),
            emit_ty(&m.return_type)
        ));
    }
    out.push_str("}\n");
    out
}

fn emit_impl(b: &TypedImplBlock) -> String {
    let mut out = format!("impl {} for {} {{\n", b.interface, b.for_type);
    for m in &b.methods {
        out.push_str(&emit_fn(m, 1));
        out.push('\n');
    }
    for h in &b.hooks {
        out.push_str(&emit_hook(h, 1));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn emit_fn(f: &TypedFnDef, indent: usize) -> String {
    let ind = indent_str(indent);
    let params: Vec<String> = f.params.iter().map(emit_param).collect();
    let body = emit_block(&f.body, indent + 1);
    let ann = if f.is_impure {
        format!("{}@impure\n", ind)
    } else {
        String::new()
    };
    format!(
        "{}{}def {}({}) -> {} {}\n",
        ann,
        ind,
        f.name,
        params.join(", "),
        emit_ty(&f.return_type),
        body
    )
}

fn emit_hook(h: &TypedHookDef, indent: usize) -> String {
    let ind = indent_str(indent);
    let op = match &h.name {
        HookName::Op(s) => s.clone(),
        HookName::Named(s) => s.clone(),
    };
    let params: Vec<String> = h.params.iter().map(emit_param).collect();
    let body = emit_block(&h.body, indent + 1);
    format!(
        "{}hook {}({}) -> {} {}\n",
        ind,
        op,
        params.join(", "),
        emit_ty(&h.return_type),
        body
    )
}

fn emit_param(p: &TypedParam) -> String {
    format!("{}: {}", p.name, emit_ty(&p.ty))
}

fn emit_block(block: &TypedBlock, indent: usize) -> String {
    if block.stmts.is_empty() {
        return "{}".to_string();
    }
    let outer = indent_str(indent.saturating_sub(1));
    let mut out = String::from("{\n");
    for stmt in &block.stmts {
        out.push_str(&emit_stmt(stmt, indent));
    }
    out.push_str(&format!("{}}}", outer));
    out
}

fn emit_stmt(stmt: &TypedStmt, indent: usize) -> String {
    let ind = indent_str(indent);
    match stmt {
        TypedStmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            ..
        } => {
            let mut_kw = if *mutable { "mut " } else { "" };
            format!(
                "{}{}{}: {} = {}\n",
                ind,
                mut_kw,
                name,
                emit_ty(ty),
                emit_expr(value)
            )
        }
        TypedStmt::Assign { target, value, .. } => {
            format!("{}{} = {}\n", ind, emit_expr(target), emit_expr(value))
        }
        TypedStmt::CompoundAssign {
            target, op, rhs, ..
        } => {
            format!(
                "{}{} {}= {}\n",
                ind,
                emit_expr(target),
                binop_str(op),
                emit_expr(rhs)
            )
        }
        TypedStmt::Return { value: Some(v), .. } => {
            format!("{}return {}\n", ind, emit_expr(v))
        }
        TypedStmt::Return { value: None, .. } => {
            format!("{}return\n", ind)
        }
        TypedStmt::Raise { value: Some(v), .. } => {
            format!("{}raise {}\n", ind, emit_expr(v))
        }
        TypedStmt::Raise { value: None, .. } => {
            format!("{}raise\n", ind)
        }
        TypedStmt::Break(_) => format!("{}break\n", ind),
        TypedStmt::Continue(_) => format!("{}continue\n", ind),
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            let mut out = String::new();
            for (i, (cond, body)) in branches.iter().enumerate() {
                let kw = if i == 0 { "if" } else { "elif" };
                out.push_str(&format!("{}{} {} ", ind, kw, emit_expr(cond)));
                out.push_str(&emit_block(body, indent + 1));
                out.push('\n');
            }
            if let Some(eb) = else_branch {
                out.push_str(&format!("{}else ", ind));
                out.push_str(&emit_block(eb, indent + 1));
                out.push('\n');
            }
            out
        }
        TypedStmt::While { cond, body, .. } => {
            format!(
                "{}while {} {}\n",
                ind,
                emit_expr(cond),
                emit_block(body, indent + 1)
            )
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            format!(
                "{}do {} while {}\n",
                ind,
                emit_block(body, indent + 1),
                emit_expr(cond)
            )
        }
        TypedStmt::For {
            binding,
            iterable,
            body,
            ..
        } => {
            format!(
                "{}for {} <- {} {}\n",
                ind,
                binding,
                emit_expr(iterable),
                emit_block(body, indent + 1)
            )
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            let mut out = format!("{}try {}\n", ind, emit_block(body, indent + 1));
            for h in handlers {
                out.push_str(&format!(
                    "{}catch {}: {} {}\n",
                    ind,
                    h.binding,
                    emit_ty(&h.ty),
                    emit_block(&h.body, indent + 1)
                ));
            }
            if let Some(f) = finally {
                out.push_str(&format!("{}finally {}\n", ind, emit_block(f, indent + 1)));
            }
            out
        }
        TypedStmt::FnDef(f) => emit_fn(f, indent),
        TypedStmt::Expr(e) => format!("{}{}\n", ind, emit_expr(e)),
    }
}

fn emit_expr(expr: &TypedExpr) -> String {
    emit_expr_inner(expr, false)
}

fn emit_expr_inner(expr: &TypedExpr, needs_parens: bool) -> String {
    let s = emit_expr_kind(&expr.kind);
    if needs_parens {
        format!("({})", s)
    } else {
        s
    }
}

fn emit_expr_kind(kind: &TypedExprKind) -> String {
    match kind {
        TypedExprKind::Int(n) => format!("{}", n),
        TypedExprKind::Float(f) => emit_float(*f),
        TypedExprKind::Bool(b) => format!("{}", b),
        TypedExprKind::Str(segs) => emit_str_segs(segs),
        TypedExprKind::Ident(name) => name.clone(),
        TypedExprKind::Tuple(elems) => {
            let parts: Vec<_> = elems.iter().map(emit_expr).collect();
            format!("({})", parts.join(", "))
        }
        TypedExprKind::StructLiteral { ty_name, fields } => {
            let parts: Vec<_> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, emit_expr(v)))
                .collect();
            format!("{} {{ {} }}", ty_name, parts.join(", "))
        }
        TypedExprKind::Call { callee, args, .. } => {
            let parts: Vec<_> = args.iter().map(emit_expr).collect();
            format!("{}({})", emit_expr(callee), parts.join(", "))
        }
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            let method_name = method_fn
                .find('_')
                .map(|i| &method_fn[i + 1..])
                .unwrap_or(method_fn.as_str());
            let parts: Vec<_> = args.iter().map(emit_expr).collect();
            format!(
                "{}.{}({})",
                emit_expr_inner(object, needs_parens_wrap(object)),
                method_name,
                parts.join(", ")
            )
        }
        TypedExprKind::StaticCall { method_fn, args } => {
            let parts: Vec<_> = args.iter().map(emit_expr).collect();
            format!("{}({})", method_fn, parts.join(", "))
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            let parts: Vec<_> = args.iter().map(emit_expr).collect();
            format!("{}({})", emit_expr(fat_ptr), parts.join(", "))
        }
        TypedExprKind::Field { object, field } => {
            format!(
                "{}.{}",
                emit_expr_inner(object, needs_parens_wrap(object)),
                field
            )
        }
        TypedExprKind::Index { object, index } => {
            format!(
                "{}[{}]",
                emit_expr_inner(object, needs_parens_wrap(object)),
                emit_expr(index)
            )
        }
        TypedExprKind::BinOp { op, left, right } => {
            let l = emit_binop_child(left, op, false);
            let r = emit_binop_child(right, op, true);
            format!("{} {} {}", l, binop_str(op), r)
        }
        TypedExprKind::UnOp { op, operand } => {
            let inner =
                needs_parens_wrap(operand) || matches!(&operand.kind, TypedExprKind::BinOp { .. });
            format!("{}{}", unop_str(op), emit_expr_inner(operand, inner))
        }
        TypedExprKind::EnumVariant {
            enum_name, variant, ..
        } => {
            format!("{}:{}", enum_name, variant)
        }
        TypedExprKind::Unwrap(inner) => {
            format!("{}!", emit_expr(inner))
        }
        TypedExprKind::As { expr, ty } => {
            format!(
                "{} as {}",
                emit_expr_inner(expr, needs_parens_wrap(expr)),
                emit_ty(ty)
            )
        }
        TypedExprKind::Match { scrutinee, arms } => emit_match(scrutinee, arms),
        TypedExprKind::Closure { params, body } => emit_closure(params, body),
        TypedExprKind::Spawn(inner) => format!("spawn {}", emit_expr(inner)),
        TypedExprKind::Ref { mutable, expr } => {
            let kw = if *mutable { "&mut " } else { "&" };
            format!("{}{}", kw, emit_expr(expr))
        }
        TypedExprKind::Array(elems) => {
            let parts: Vec<_> = elems.iter().map(emit_expr).collect();
            format!("[{}]", parts.join(", "))
        }
        TypedExprKind::BoundMethod {
            object,
            qualified_name,
        } => {
            format!("{}.{}", emit_expr(object), qualified_name)
        }
        TypedExprKind::PrimTypeRef { target, .. } => format!("<{:?}>", target),
        TypedExprKind::Gen { .. } => "<gen>".to_string(),
        TypedExprKind::GenSplice(inner) => format!("splice {}", emit_expr(inner)),
        TypedExprKind::Block(stmts) => {
            let body: String = stmts.iter().map(|s| emit_stmt(s, 1)).collect();
            format!("{{\n{}}}", body)
        }
    }
}

fn needs_parens_wrap(expr: &TypedExpr) -> bool {
    matches!(
        &expr.kind,
        TypedExprKind::BinOp { .. } | TypedExprKind::As { .. }
    )
}

fn emit_binop_child(expr: &TypedExpr, parent_op: &BinOp, is_right: bool) -> String {
    match &expr.kind {
        TypedExprKind::BinOp { op: child_op, .. } => {
            if binop_prec(child_op) < binop_prec(parent_op)
                || (is_right && binop_prec(child_op) == binop_prec(parent_op))
            {
                format!("({})", emit_expr_kind(&expr.kind))
            } else {
                emit_expr_kind(&expr.kind)
            }
        }
        _ => emit_expr(expr),
    }
}

fn binop_prec(op: &BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq
        | BinOp::Ne
        | BinOp::Lt
        | BinOp::Gt
        | BinOp::LtEq
        | BinOp::GtEq
        | BinOp::Spaceship => 3,
        BinOp::Pipe => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

fn binop_str(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::LtEq => "<=",
        BinOp::GtEq => ">=",
        BinOp::Spaceship => "<=>",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Pipe => "|",
    }
}

fn unop_str(op: &UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Pos => "+",
    }
}

fn emit_float(f: f64) -> String {
    let s = format!("{}", f);
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

fn emit_str_segs(segs: &[TypedStringSegment]) -> String {
    let mut inner = String::new();
    for seg in segs {
        match seg {
            TypedStringSegment::Text(t) => inner.push_str(t),
            TypedStringSegment::Interp(e) => {
                inner.push('{');
                inner.push_str(&emit_expr(e));
                inner.push('}');
            }
        }
    }
    format!("\"{}\"", inner)
}

fn emit_match(scrutinee: &TypedExpr, arms: &[TypedMatchArm]) -> String {
    let mut out = format!("match {} {{\n", emit_expr(scrutinee));
    for arm in arms {
        let pat = emit_pattern(&arm.pattern);
        let guard = arm
            .guard
            .as_ref()
            .map(|g| format!(" if {}", emit_expr(g)))
            .unwrap_or_default();
        out.push_str(&format!(
            "    {}{} => {},\n",
            pat,
            guard,
            emit_expr(&arm.body)
        ));
    }
    out.push('}');
    out
}

fn emit_pattern(pat: &TypedPattern) -> String {
    match pat {
        TypedPattern::Wildcard(_) => "_".to_string(),
        TypedPattern::Literal(e) => emit_expr(e),
        TypedPattern::TypeBinding { ty, name, .. } => format!("{} {}", ty, name),
        TypedPattern::InterfaceGuard {
            interface, name, ..
        } => format!("{}: {}", name, interface),
        TypedPattern::Struct {
            variant, fields, ..
        } => {
            let parts: Vec<_> = fields
                .iter()
                .map(|(f, b)| format!("{}: {}", f, b))
                .collect();
            format!("{} {{ {} }}", variant, parts.join(", "))
        }
        TypedPattern::Tuple(pats, _) => {
            let parts: Vec<_> = pats.iter().map(emit_pattern).collect();
            format!("({})", parts.join(", "))
        }
    }
}

fn emit_closure(params: &[TypedParam], body: &TypedClosureBody) -> String {
    let params_str: Vec<_> = params.iter().map(emit_param).collect();
    let body_str = match body {
        TypedClosureBody::Expr(e) => emit_expr(e),
        TypedClosureBody::Block(b) => emit_block(b, 1),
    };
    format!("|{}| {}", params_str.join(", "), body_str)
}
