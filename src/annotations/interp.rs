use crate::diagnostics::Span;
use crate::parser::ast::{
    BinOp, Block, Expr, FnDef, Item, ProcessorDef, Stmt, StringSegment, TypeExpr, UnOp,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Runtime values for the processor interpreter
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Void,
    None,
    Some(Box<Value>),
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Block(Block),
    Decl(Item),
    /// Generic struct value backing @builtin structs (FnDecl, AnnotArgs, etc.)
    Struct {
        ty: String,
        fields: HashMap<String, Value>,
    },
}

// ---------------------------------------------------------------------------
// Interpreter
// ---------------------------------------------------------------------------

pub struct Interpreter {
    env: HashMap<String, Value>,
}

enum StmtOutcome {
    Return(Value),
    Break,
    Value(Value),
    Void,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            env: HashMap::new(),
        }
    }

    pub fn bind(&mut self, name: impl Into<String>, val: Value) {
        self.env.insert(name.into(), val);
    }

    // Evaluate annotation arg exprs into a map of name -> Value.
    pub fn eval_annot_args(args: &[(String, Expr)]) -> HashMap<String, Value> {
        let mut interp = Interpreter::new();
        let mut map = HashMap::new();
        for (name, expr) in args {
            if let Ok(v) = interp.eval_expr(expr) {
                map.insert(name.clone(), v);
            }
        }
        map
    }

    pub fn run_processor(
        fn_def: &FnDef,
        proc: &ProcessorDef,
        annot_args: &[(String, Expr)],
    ) -> Option<(Option<Item>, Vec<Item>)> {
        let mut interp = Interpreter::new();
        let args_map = Interpreter::eval_annot_args(annot_args);

        let mut fn_fields: HashMap<String, Value> = HashMap::new();
        fn_fields.insert("name".into(), Value::Str(fn_def.name.clone()));
        fn_fields.insert(
            "return_type".into(),
            Value::Str(type_expr_to_str(&fn_def.return_type)),
        );
        fn_fields.insert("body".into(), Value::Block(fn_def.body.clone()));
        fn_fields.insert(
            "annot".into(),
            Value::Struct {
                ty: "AnnotArgs".into(),
                fields: args_map,
            },
        );
        let params_list: Vec<Value> = fn_def
            .params
            .iter()
            .map(|p| {
                let mut pf = HashMap::new();
                pf.insert("name".into(), Value::Str(p.name.clone()));
                pf.insert("ty".into(), Value::Str(type_expr_to_str(&p.ty)));
                Value::Struct {
                    ty: "Param".into(),
                    fields: pf,
                }
            })
            .collect();
        fn_fields.insert("params".into(), Value::List(params_list));
        // Keep a copy of the original FnDef for with_body/with_name mutations.
        fn_fields.insert(
            "__fn_def__".into(),
            Value::Decl(Item::Function(fn_def.clone())),
        );

        let target = Value::Struct {
            ty: "FnDecl".into(),
            fields: fn_fields,
        };
        interp.bind(&proc.target_param.name, target);

        let result = interp.eval_block(&proc.body).ok()?;
        decode_processor_result(result)
    }

    fn eval_block(&mut self, block: &Block) -> Result<Value, ()> {
        match self.eval_block_inner(block)? {
            StmtOutcome::Return(v) | StmtOutcome::Value(v) => Ok(v),
            StmtOutcome::Break | StmtOutcome::Void => Ok(Value::Void),
        }
    }

    // Like eval_block but propagates Return and Break so nested blocks can
    // short-circuit the enclosing function or loop.
    fn eval_block_inner(&mut self, block: &Block) -> Result<StmtOutcome, ()> {
        let mut last = StmtOutcome::Void;
        for stmt in &block.stmts {
            let outcome = self.eval_stmt(stmt)?;
            match outcome {
                StmtOutcome::Return(_) | StmtOutcome::Break => return Ok(outcome),
                StmtOutcome::Value(_) => last = outcome,
                StmtOutcome::Void => {}
            }
        }
        Ok(last)
    }

    fn eval_stmt(&mut self, stmt: &Stmt) -> Result<StmtOutcome, ()> {
        match stmt {
            Stmt::Return { value: Some(e), .. } => Ok(StmtOutcome::Return(self.eval_expr(e)?)),
            Stmt::Return { value: None, .. } => Ok(StmtOutcome::Return(Value::Void)),
            Stmt::Break(_) => Ok(StmtOutcome::Break),
            Stmt::Continue(_) => Ok(StmtOutcome::Void),
            Stmt::VarDecl { name, value, .. } => {
                let v = self.eval_expr(value)?;
                self.env.insert(name.clone(), v);
                Ok(StmtOutcome::Void)
            }
            Stmt::Assign {
                target: Expr::Ident(name, _),
                value,
                ..
            } => {
                let v = self.eval_expr(value)?;
                self.env.insert(name.clone(), v);
                Ok(StmtOutcome::Void)
            }
            Stmt::CompoundAssign {
                target: Expr::Ident(name, _),
                op,
                rhs,
                ..
            } => {
                let current = self.lookup(name)?;
                let rhs_val = self.eval_expr(rhs)?;
                let new_val = eval_binop(op, current, rhs_val)?;
                self.env.insert(name.clone(), new_val);
                Ok(StmtOutcome::Void)
            }
            Stmt::If {
                branches,
                else_branch,
                ..
            } => {
                for (cond, body) in branches {
                    let cv = self.eval_expr(cond)?;
                    if is_truthy(&cv) {
                        return self.eval_block_inner(body);
                    }
                }
                if let Some(else_b) = else_branch {
                    return self.eval_block_inner(else_b);
                }
                Ok(StmtOutcome::Void)
            }
            Stmt::While { cond, body, .. } => {
                loop {
                    let cv = self.eval_expr(cond)?;
                    if !is_truthy(&cv) {
                        break;
                    }
                    match self.eval_block_inner(body)? {
                        StmtOutcome::Return(v) => return Ok(StmtOutcome::Return(v)),
                        StmtOutcome::Break => break,
                        _ => {}
                    }
                }
                Ok(StmtOutcome::Void)
            }
            Stmt::For {
                binding,
                iterable,
                body,
                ..
            } => {
                let items = match self.eval_expr(iterable)? {
                    Value::List(v) => v,
                    _ => return Err(()),
                };
                for item in items {
                    self.env.insert(binding.clone(), item);
                    match self.eval_block_inner(body)? {
                        StmtOutcome::Return(v) => return Ok(StmtOutcome::Return(v)),
                        StmtOutcome::Break => break,
                        _ => {}
                    }
                }
                Ok(StmtOutcome::Void)
            }
            Stmt::Expr(e) => {
                let v = self.eval_expr(e)?;
                Ok(StmtOutcome::Value(v))
            }
            _ => Ok(StmtOutcome::Void),
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, ()> {
        match expr {
            Expr::Int(n, _) => Ok(Value::Int(*n)),
            Expr::Float(f, _) => Ok(Value::Float(*f)),
            Expr::Bool(b, _) => Ok(Value::Bool(*b)),
            Expr::Str(segs, _) => {
                let mut parts: Vec<String> = Vec::new();
                for seg in segs {
                    match seg {
                        StringSegment::Text(t) => parts.push(t.clone()),
                        StringSegment::Interp(e) => match self.eval_expr(e)? {
                            Value::Str(s) => parts.push(s),
                            Value::Int(n) => parts.push(n.to_string()),
                            Value::Float(f) => parts.push(f.to_string()),
                            Value::Bool(b) => parts.push(b.to_string()),
                            _ => return Err(()),
                        },
                    }
                }
                Ok(Value::Str(parts.join("")))
            }
            Expr::Ident(name, _) => self.lookup(name),
            Expr::Tuple(elems, _) => {
                let vals: Result<Vec<_>, _> = elems.iter().map(|e| self.eval_expr(e)).collect();
                Ok(Value::Tuple(vals?))
            }
            Expr::Array(elems, _) => {
                let vals: Result<Vec<_>, _> = elems.iter().map(|e| self.eval_expr(e)).collect();
                Ok(Value::List(vals?))
            }
            Expr::StructLiteral { ty, fields, .. } if ty == "Some" => {
                let (_, val_expr) = fields.iter().find(|(n, _)| n == "value").ok_or(())?;
                Ok(Value::Some(Box::new(self.eval_expr(val_expr)?)))
            }
            Expr::Field { object, field, .. } => {
                let obj = self.eval_expr(object)?;
                eval_field(obj, field)
            }
            Expr::Call { callee, args, .. } => self.eval_call(callee, args),
            Expr::Gen { body, .. } => self.eval_gen_block(body),
            Expr::BinOp {
                op, left, right, ..
            } => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                eval_binop(op, lv, rv)
            }
            Expr::UnOp { op, operand, .. } => {
                let v = self.eval_expr(operand)?;
                eval_unop(op, v)
            }
            _ => Err(()),
        }
    }

    fn lookup(&self, name: &str) -> Result<Value, ()> {
        match name {
            "None" => Ok(Value::None),
            _ => self.env.get(name).cloned().ok_or(()),
        }
    }

    fn eval_call(&mut self, callee: &Expr, raw_args: &[Expr]) -> Result<Value, ()> {
        // Evaluate args first (needed for both instance and static calls).
        let args: Result<Vec<_>, _> = raw_args.iter().map(|a| self.eval_expr(a)).collect();
        let args = args?;

        match callee {
            Expr::Field { object, field, .. } => {
                // Try static call first (e.g. Vec.new())
                if let Expr::Ident(type_name, _) = object.as_ref() {
                    if let Ok(v) = eval_static_method(type_name, field, &args) {
                        return Ok(v);
                    }
                }
                // Instance method call
                let obj = self.eval_expr(object)?;
                eval_method(obj, field, args)
            }
            _ => Err(()),
        }
    }

    fn eval_gen_block(&mut self, block: &Block) -> Result<Value, ()> {
        let zero = Span::new(0, 0);
        let mut stmts: Vec<Stmt> = vec![];

        for stmt in &block.stmts {
            // Top-level splice: <<expr>> as a standalone statement
            if let Stmt::Expr(Expr::GenSplice(inner, _)) = stmt {
                let val = self.eval_expr(inner)?;
                splice_into(&mut stmts, val, zero);
                continue;
            }
            // Otherwise substitute splices inside the statement
            stmts.push(self.subst_stmt(stmt)?);
        }

        Ok(Value::Block(Block { stmts, span: zero }))
    }

    fn subst_stmt(&mut self, stmt: &Stmt) -> Result<Stmt, ()> {
        match stmt {
            Stmt::VarDecl {
                name,
                ty,
                value,
                mutable,
                span,
            } => Ok(Stmt::VarDecl {
                name: name.clone(),
                ty: ty.clone(),
                value: self.subst_expr(value)?,
                mutable: *mutable,
                span: *span,
            }),
            Stmt::Assign {
                target,
                value,
                span,
            } => Ok(Stmt::Assign {
                target: self.subst_expr(target)?,
                value: self.subst_expr(value)?,
                span: *span,
            }),
            Stmt::CompoundAssign {
                target,
                op,
                rhs,
                span,
            } => Ok(Stmt::CompoundAssign {
                target: self.subst_expr(target)?,
                op: op.clone(),
                rhs: self.subst_expr(rhs)?,
                span: *span,
            }),
            Stmt::Return {
                value: Some(e),
                span,
            } => Ok(Stmt::Return {
                value: Some(self.subst_expr(e)?),
                span: *span,
            }),
            Stmt::Raise {
                value: Some(e),
                span,
            } => Ok(Stmt::Raise {
                value: Some(self.subst_expr(e)?),
                span: *span,
            }),
            Stmt::Expr(e) => Ok(Stmt::Expr(self.subst_expr(e)?)),
            Stmt::While { cond, body, span } => Ok(Stmt::While {
                cond: self.subst_expr(cond)?,
                body: self.subst_block(body)?,
                span: *span,
            }),
            Stmt::DoWhile { body, cond, span } => Ok(Stmt::DoWhile {
                body: self.subst_block(body)?,
                cond: self.subst_expr(cond)?,
                span: *span,
            }),
            Stmt::For {
                binding,
                binding_ty,
                iterable,
                body,
                span,
            } => Ok(Stmt::For {
                binding: binding.clone(),
                binding_ty: binding_ty.clone(),
                iterable: self.subst_expr(iterable)?,
                body: self.subst_block(body)?,
                span: *span,
            }),
            Stmt::If {
                branches,
                else_branch,
                span,
            } => {
                let new_branches: Result<Vec<_>, _> = branches
                    .iter()
                    .map(|(c, b)| Ok((self.subst_expr(c)?, self.subst_block(b)?)))
                    .collect();
                let new_else = else_branch
                    .as_ref()
                    .map(|b| self.subst_block(b))
                    .transpose()?;
                Ok(Stmt::If {
                    branches: new_branches?,
                    else_branch: new_else,
                    span: *span,
                })
            }
            Stmt::TryCatch {
                body,
                handlers,
                finally,
                span,
            } => {
                let new_handlers: Result<Vec<_>, _> = handlers
                    .iter()
                    .map(|h| {
                        Ok(crate::parser::ast::CatchHandler {
                            ty: h.ty.clone(),
                            binding: h.binding.clone(),
                            body: self.subst_block(&h.body)?,
                            span: h.span,
                        })
                    })
                    .collect();
                let new_finally = finally.as_ref().map(|b| self.subst_block(b)).transpose()?;
                Ok(Stmt::TryCatch {
                    body: self.subst_block(body)?,
                    handlers: new_handlers?,
                    finally: new_finally,
                    span: *span,
                })
            }
            _ => Ok(stmt.clone()),
        }
    }

    fn subst_block(&mut self, block: &Block) -> Result<Block, ()> {
        let mut stmts: Vec<Stmt> = Vec::new();
        for stmt in &block.stmts {
            if let Stmt::Expr(Expr::GenSplice(inner, _)) = stmt {
                let val = self.eval_expr(inner)?;
                let span = inner.span();
                match val {
                    Value::Block(b) => stmts.extend(b.stmts),
                    other => stmts.push(Stmt::Expr(value_to_expr(other, span)?)),
                }
                continue;
            }
            stmts.push(self.subst_stmt(stmt)?);
        }
        Ok(Block {
            stmts,
            span: block.span,
        })
    }

    fn subst_expr(&mut self, expr: &Expr) -> Result<Expr, ()> {
        match expr {
            Expr::GenSplice(inner, span) => {
                let val = self.eval_expr(inner)?;
                value_to_expr(val, *span)
            }
            Expr::Str(segs, span) => {
                let new_segs: Result<Vec<_>, _> = segs
                    .iter()
                    .map(|seg| match seg {
                        StringSegment::Text(_) => Ok(seg.clone()),
                        StringSegment::Interp(e) => Ok(StringSegment::Interp(self.subst_expr(e)?)),
                    })
                    .collect();
                Ok(Expr::Str(new_segs?, *span))
            }
            Expr::BinOp {
                op,
                left,
                right,
                span,
            } => Ok(Expr::BinOp {
                op: op.clone(),
                left: Box::new(self.subst_expr(left)?),
                right: Box::new(self.subst_expr(right)?),
                span: *span,
            }),
            Expr::Call { callee, args, span } => {
                let new_args: Result<Vec<_>, _> = args.iter().map(|a| self.subst_expr(a)).collect();
                Ok(Expr::Call {
                    callee: Box::new(self.subst_expr(callee)?),
                    args: new_args?,
                    span: *span,
                })
            }
            Expr::Field {
                object,
                field,
                span,
            } => Ok(Expr::Field {
                object: Box::new(self.subst_expr(object)?),
                field: field.clone(),
                span: *span,
            }),
            Expr::Index {
                object,
                index,
                span,
            } => Ok(Expr::Index {
                object: Box::new(self.subst_expr(object)?),
                index: Box::new(self.subst_expr(index)?),
                span: *span,
            }),
            Expr::UnOp { op, operand, span } => Ok(Expr::UnOp {
                op: op.clone(),
                operand: Box::new(self.subst_expr(operand)?),
                span: *span,
            }),
            Expr::Unwrap(e, span) => Ok(Expr::Unwrap(Box::new(self.subst_expr(e)?), *span)),
            Expr::Tuple(elems, span) => {
                let new_elems: Result<Vec<_>, _> =
                    elems.iter().map(|e| self.subst_expr(e)).collect();
                Ok(Expr::Tuple(new_elems?, *span))
            }
            Expr::Array(elems, span) => {
                let new_elems: Result<Vec<_>, _> =
                    elems.iter().map(|e| self.subst_expr(e)).collect();
                Ok(Expr::Array(new_elems?, *span))
            }
            Expr::StructLiteral { ty, fields, span } => {
                let new_fields: Result<Vec<_>, _> = fields
                    .iter()
                    .map(|(n, e)| Ok((n.clone(), self.subst_expr(e)?)))
                    .collect();
                Ok(Expr::StructLiteral {
                    ty: ty.clone(),
                    fields: new_fields?,
                    span: *span,
                })
            }
            Expr::As { expr, ty, span } => Ok(Expr::As {
                expr: Box::new(self.subst_expr(expr)?),
                ty: ty.clone(),
                span: *span,
            }),
            Expr::Spawn(e, span) => Ok(Expr::Spawn(Box::new(self.subst_expr(e)?), *span)),
            Expr::Ref {
                mutable,
                expr,
                span,
            } => Ok(Expr::Ref {
                mutable: *mutable,
                expr: Box::new(self.subst_expr(expr)?),
                span: *span,
            }),
            Expr::Match {
                scrutinee,
                arms,
                span,
            } => {
                let new_arms: Result<Vec<_>, _> = arms
                    .iter()
                    .map(|arm| {
                        Ok(crate::parser::ast::MatchArm {
                            pattern: arm.pattern.clone(),
                            guard: arm.guard.as_ref().map(|g| self.subst_expr(g)).transpose()?,
                            body: self.subst_expr(&arm.body)?,
                            span: arm.span,
                        })
                    })
                    .collect();
                Ok(Expr::Match {
                    scrutinee: Box::new(self.subst_expr(scrutinee)?),
                    arms: new_arms?,
                    span: *span,
                })
            }
            _ => Ok(expr.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers (no &mut self access needed)
// ---------------------------------------------------------------------------

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int(n) => *n != 0,
        _ => false,
    }
}

fn eval_binop(op: &BinOp, lv: Value, rv: Value) -> Result<Value, ()> {
    match (op, lv, rv) {
        (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (BinOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
        (BinOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
        (BinOp::Div, Value::Int(a), Value::Int(b)) if b != 0 => Ok(Value::Int(a / b)),
        (BinOp::Mod, Value::Int(a), Value::Int(b)) if b != 0 => Ok(Value::Int(a % b)),
        (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (BinOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        (BinOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        (BinOp::Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(a + &b)),
        (BinOp::Eq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
        (BinOp::Ne, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a != b)),
        (BinOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (BinOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (BinOp::LtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
        (BinOp::GtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
        (BinOp::Eq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a == b)),
        (BinOp::Ne, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a != b)),
        (BinOp::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
        (BinOp::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        (BinOp::LtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
        (BinOp::GtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
        (BinOp::Eq, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
        (BinOp::Ne, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a != b)),
        (BinOp::And, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a && b)),
        (BinOp::Or, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a || b)),
        (BinOp::Eq, Value::Str(a), Value::Str(b)) => Ok(Value::Bool(a == b)),
        (BinOp::Ne, Value::Str(a), Value::Str(b)) => Ok(Value::Bool(a != b)),
        _ => Err(()),
    }
}

fn eval_unop(op: &UnOp, v: Value) -> Result<Value, ()> {
    match (op, v) {
        (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
        (UnOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        _ => Err(()),
    }
}

fn eval_field(val: Value, field: &str) -> Result<Value, ()> {
    match val {
        Value::Struct { ref fields, .. } => fields.get(field).cloned().ok_or(()),
        _ => Err(()),
    }
}

fn eval_static_method(type_name: &str, method: &str, args: &[Value]) -> Result<Value, ()> {
    match (type_name, method) {
        ("Vec", "new") if args.is_empty() => Ok(Value::List(vec![])),
        ("Block", "empty") if args.is_empty() => Ok(Value::Block(Block {
            stmts: vec![],
            span: Span::new(0, 0),
        })),
        _ => Err(()),
    }
}

fn eval_method(obj: Value, method: &str, args: Vec<Value>) -> Result<Value, ()> {
    match (&obj, method) {
        (Value::Struct { ty, fields }, "with_body") if ty == "FnDecl" => {
            if args.len() != 1 {
                return Err(());
            }
            if let Value::Block(new_body) = &args[0] {
                // Recover the original FnDef from the hidden __fn_def__ field.
                if let Some(Value::Decl(Item::Function(ref orig))) = fields.get("__fn_def__") {
                    let mut new_fn = orig.clone();
                    new_fn.body = new_body.clone();
                    return Ok(Value::Decl(Item::Function(new_fn)));
                }
            }
            Err(())
        }
        (Value::Struct { ty, fields }, "with_name") if ty == "FnDecl" => {
            if args.len() != 1 {
                return Err(());
            }
            if let Value::Str(new_name) = &args[0] {
                if let Some(Value::Decl(Item::Function(ref orig))) = fields.get("__fn_def__") {
                    let mut new_fn = orig.clone();
                    new_fn.name = new_name.clone();
                    return Ok(Value::Decl(Item::Function(new_fn)));
                }
            }
            Err(())
        }
        (Value::List(_), "push") if args.len() == 1 => {
            if let Value::List(mut items) = obj {
                items.push(args.into_iter().next().unwrap());
                Ok(Value::List(items))
            } else {
                unreachable!()
            }
        }
        (Value::List(_), "new") if args.is_empty() => Ok(Value::List(vec![])),
        (Value::List(items), "len") if args.is_empty() => Ok(Value::Int(items.len() as i64)),
        (Value::Block(_), "concat") if args.len() == 1 => {
            if let (Value::Block(mut self_block), Value::Block(other_block)) =
                (obj, args.into_iter().next().unwrap())
            {
                self_block.stmts.extend(other_block.stmts);
                Ok(Value::Block(self_block))
            } else {
                unreachable!()
            }
        }
        (Value::Block(_), "prepend") if args.len() == 1 => {
            if let (Value::Block(self_block), Value::Block(mut other_block)) =
                (obj, args.into_iter().next().unwrap())
            {
                other_block.stmts.extend(self_block.stmts);
                Ok(Value::Block(other_block))
            } else {
                unreachable!()
            }
        }
        _ => Err(()),
    }
}

fn splice_into(stmts: &mut Vec<Stmt>, val: Value, span: Span) {
    match val {
        Value::Block(b) => stmts.extend(b.stmts),
        Value::Int(n) => stmts.push(Stmt::Expr(Expr::Int(n, span))),
        Value::Str(s) => stmts.push(Stmt::Expr(Expr::Str(vec![StringSegment::Text(s)], span))),
        Value::Bool(b) => stmts.push(Stmt::Expr(Expr::Bool(b, span))),
        _ => {}
    }
}

fn value_to_expr(val: Value, span: Span) -> Result<Expr, ()> {
    match val {
        Value::Int(n) => Ok(Expr::Int(n, span)),
        Value::Float(f) => Ok(Expr::Float(f, span)),
        Value::Bool(b) => Ok(Expr::Bool(b, span)),
        Value::Str(s) => Ok(Expr::Str(vec![StringSegment::Text(s)], span)),
        _ => Err(()),
    }
}

fn type_expr_to_str(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Named { name, .. } => name.clone(),
        _ => "unknown".into(),
    }
}

fn decode_processor_result(val: Value) -> Option<(Option<Item>, Vec<Item>)> {
    if let Value::Tuple(parts) = val {
        if parts.len() != 2 {
            return None;
        }
        let opt_item = match &parts[0] {
            Value::None => None,
            Value::Some(inner) => {
                if let Value::Decl(item) = inner.as_ref() {
                    Some(item.clone())
                } else {
                    None
                }
            }
            _ => None,
        };
        let extras: Vec<Item> = match &parts[1] {
            Value::List(items) => items
                .iter()
                .filter_map(|v| {
                    if let Value::Decl(item) = v {
                        Some(item.clone())
                    } else {
                        None
                    }
                })
                .collect(),
            _ => vec![],
        };
        Some((opt_item, extras))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::*;

    fn zero() -> Span {
        Span::new(0, 0)
    }

    fn simple_fn(name: &str, stmts: Vec<Stmt>) -> FnDef {
        FnDef {
            annotations: vec![],
            name: name.into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: TypeExpr::Named {
                name: "void".into(),
                generics: vec![],
                bindings: vec![],
                span: zero(),
            },
            body: Block {
                stmts,
                span: zero(),
            },
            is_declaration: false,
            span: zero(),
        }
    }

    fn void_stmt() -> Stmt {
        Stmt::Expr(Expr::Int(0, zero()))
    }

    #[test]
    fn eval_int_literal() {
        let mut interp = Interpreter::new();
        let v = interp.eval_expr(&Expr::Int(42, zero())).unwrap();
        assert!(matches!(v, Value::Int(42)));
    }

    #[test]
    fn eval_tuple_of_none_and_empty_list() {
        let mut interp = Interpreter::new();
        let expr = Expr::Tuple(
            vec![
                Expr::Ident("None".into(), zero()),
                Expr::Array(vec![], zero()),
            ],
            zero(),
        );
        let v = interp.eval_expr(&expr).unwrap();
        assert!(matches!(v, Value::Tuple(_)));
        if let Value::Tuple(parts) = v {
            assert!(matches!(parts[0], Value::None));
            assert!(matches!(parts[1], Value::List(_)));
        }
    }

    #[test]
    fn gen_empty_block_yields_empty_block() {
        let mut interp = Interpreter::new();
        let block = Block {
            stmts: vec![],
            span: zero(),
        };
        let v = interp.eval_gen_block(&block).unwrap();
        assert!(matches!(v, Value::Block(ref b) if b.stmts.is_empty()));
    }

    #[test]
    fn struct_field_access_returns_value() {
        let mut fields = HashMap::new();
        fields.insert(
            "body".into(),
            Value::Block(Block {
                stmts: vec![],
                span: zero(),
            }),
        );
        fields.insert("name".into(), Value::Str("foo".into()));
        let target = Value::Struct {
            ty: "FnDecl".into(),
            fields,
        };
        let mut interp = Interpreter::new();
        interp.bind("target", target);

        let expr = Expr::Field {
            object: Box::new(Expr::Ident("target".into(), zero())),
            field: "name".into(),
            span: zero(),
        };
        let v = interp.eval_expr(&expr).unwrap();
        assert!(matches!(v, Value::Str(ref s) if s == "foo"));
    }

    #[test]
    fn gen_block_splice_of_block_extends_stmts() {
        let fn_def = simple_fn("foo", vec![void_stmt()]);
        let mut fn_fields: HashMap<String, Value> = HashMap::new();
        fn_fields.insert("name".into(), Value::Str("foo".into()));
        fn_fields.insert("return_type".into(), Value::Str("void".into()));
        fn_fields.insert("body".into(), Value::Block(fn_def.body.clone()));
        fn_fields.insert(
            "annot".into(),
            Value::Struct {
                ty: "AnnotArgs".into(),
                fields: HashMap::new(),
            },
        );
        fn_fields.insert("__fn_def__".into(), Value::Decl(Item::Function(fn_def)));
        let target = Value::Struct {
            ty: "FnDecl".into(),
            fields: fn_fields,
        };

        let mut interp = Interpreter::new();
        interp.bind("target", target);

        // gen { <<target.body>> }
        let splice_expr = Expr::Field {
            object: Box::new(Expr::Ident("target".into(), zero())),
            field: "body".into(),
            span: zero(),
        };
        let block = Block {
            stmts: vec![Stmt::Expr(Expr::GenSplice(Box::new(splice_expr), zero()))],
            span: zero(),
        };
        let v = interp.eval_gen_block(&block).unwrap();
        if let Value::Block(b) = v {
            assert_eq!(b.stmts.len(), 1, "splice of 1-stmt body yields 1 stmt");
        } else {
            panic!("expected Block");
        }
    }

    #[test]
    fn gen_block_prepends_stmt_before_splice() {
        let fn_def = simple_fn("foo", vec![void_stmt()]);
        let mut fn_fields: HashMap<String, Value> = HashMap::new();
        fn_fields.insert("name".into(), Value::Str("foo".into()));
        fn_fields.insert("return_type".into(), Value::Str("void".into()));
        fn_fields.insert("body".into(), Value::Block(fn_def.body.clone()));
        fn_fields.insert(
            "annot".into(),
            Value::Struct {
                ty: "AnnotArgs".into(),
                fields: HashMap::new(),
            },
        );
        fn_fields.insert("__fn_def__".into(), Value::Decl(Item::Function(fn_def)));
        let target = Value::Struct {
            ty: "FnDecl".into(),
            fields: fn_fields,
        };

        let mut interp = Interpreter::new();
        interp.bind("target", target);

        // gen { sentinel: int = 0; <<target.body>> }
        let splice_expr = Expr::Field {
            object: Box::new(Expr::Ident("target".into(), zero())),
            field: "body".into(),
            span: zero(),
        };
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "sentinel".into(),
                    ty: TypeExpr::Named {
                        name: "int".into(),
                        generics: vec![],
                        bindings: vec![],
                        span: zero(),
                    },
                    value: Expr::Int(0, zero()),
                    mutable: false,
                    span: zero(),
                },
                Stmt::Expr(Expr::GenSplice(Box::new(splice_expr), zero())),
            ],
            span: zero(),
        };
        let v = interp.eval_gen_block(&block).unwrap();
        if let Value::Block(b) = v {
            assert_eq!(b.stmts.len(), 2, "prepend + 1 original stmt = 2 stmts");
        } else {
            panic!("expected Block");
        }
    }

    #[test]
    fn block_empty_has_no_stmts() {
        let v = eval_static_method("Block", "empty", &[]).unwrap();
        assert!(matches!(v, Value::Block(ref b) if b.stmts.is_empty()));
    }

    #[test]
    fn block_concat_empty_with_nonempty_yields_nonempty() {
        let empty = eval_static_method("Block", "empty", &[]).unwrap();
        let nonempty = Value::Block(Block {
            stmts: vec![void_stmt()],
            span: zero(),
        });
        let result = eval_method(empty, "concat", vec![nonempty]).unwrap();
        assert!(matches!(result, Value::Block(ref b) if b.stmts.len() == 1));
    }

    #[test]
    fn block_concat_two_nonempty_yields_sum_of_stmts() {
        let a = Value::Block(Block {
            stmts: vec![void_stmt(), void_stmt()],
            span: zero(),
        });
        let b = Value::Block(Block {
            stmts: vec![void_stmt(), void_stmt(), void_stmt()],
            span: zero(),
        });
        let result = eval_method(a, "concat", vec![b]).unwrap();
        assert!(matches!(result, Value::Block(ref bl) if bl.stmts.len() == 5));
    }

    #[test]
    fn block_prepend_is_reverse_concat() {
        let a = Value::Block(Block {
            stmts: vec![Stmt::Expr(Expr::Int(1, zero()))],
            span: zero(),
        });
        let b = Value::Block(Block {
            stmts: vec![Stmt::Expr(Expr::Int(2, zero()))],
            span: zero(),
        });
        // a.prepend(b) == b.concat(a): result should be [2, 1]
        let prepend_result = eval_method(a.clone(), "prepend", vec![b.clone()]).unwrap();
        let concat_result = eval_method(b, "concat", vec![a]).unwrap();
        if let (Value::Block(pr), Value::Block(cr)) = (prepend_result, concat_result) {
            assert_eq!(pr.stmts.len(), cr.stmts.len());
            // Both should be [2, 1]: first stmt is Int(2), second is Int(1).
            assert!(matches!(&pr.stmts[0], Stmt::Expr(Expr::Int(2, _))));
            assert!(matches!(&pr.stmts[1], Stmt::Expr(Expr::Int(1, _))));
            assert!(matches!(&cr.stmts[0], Stmt::Expr(Expr::Int(2, _))));
            assert!(matches!(&cr.stmts[1], Stmt::Expr(Expr::Int(1, _))));
        } else {
            panic!("expected Block values");
        }
    }

    #[test]
    fn processor_generates_one_stmt_per_param_via_concat() {
        // Build a FnDef with 3 params and run a processor that loops over
        // target.params and concat-s one gen block per param.
        let int_ty = || TypeExpr::Named {
            name: "int".into(),
            generics: vec![],
            bindings: vec![],
            span: zero(),
        };
        let fn_def = FnDef {
            annotations: vec![],
            name: "multi".into(),
            generic_params: vec![],
            params: vec![
                crate::parser::ast::Param {
                    name: "a".into(),
                    ty: int_ty(),
                    span: zero(),
                },
                crate::parser::ast::Param {
                    name: "b".into(),
                    ty: int_ty(),
                    span: zero(),
                },
                crate::parser::ast::Param {
                    name: "c".into(),
                    ty: int_ty(),
                    span: zero(),
                },
            ],
            variadic: None,
            return_type: TypeExpr::Named {
                name: "void".into(),
                generics: vec![],
                bindings: vec![],
                span: zero(),
            },
            body: Block {
                stmts: vec![],
                span: zero(),
            },
            is_declaration: false,
            span: zero(),
        };

        // Processor body (as AST):
        //   checks: Block = Block.empty()
        //   for p <- target.params {
        //       checks = checks.concat(gen { 0 })
        //   }
        //   return (Some { value: target.with_body(checks) }, Vec.new())
        let proc_body = Block {
            stmts: vec![
                // checks: Block = Block.empty()
                Stmt::VarDecl {
                    name: "checks".into(),
                    ty: TypeExpr::Named {
                        name: "Block".into(),
                        generics: vec![],
                        bindings: vec![],
                        span: zero(),
                    },
                    value: Expr::Call {
                        callee: Box::new(Expr::Field {
                            object: Box::new(Expr::Ident("Block".into(), zero())),
                            field: "empty".into(),
                            span: zero(),
                        }),
                        args: vec![],
                        span: zero(),
                    },
                    mutable: true,
                    span: zero(),
                },
                // for p <- target.params { checks = checks.concat(gen { 0 }) }
                Stmt::For {
                    binding: "p".into(),
                    binding_ty: None,
                    iterable: Expr::Field {
                        object: Box::new(Expr::Ident("target".into(), zero())),
                        field: "params".into(),
                        span: zero(),
                    },
                    body: Block {
                        stmts: vec![
                            // checks = checks.concat(gen { 0 })
                            Stmt::Assign {
                                target: Expr::Ident("checks".into(), zero()),
                                value: Expr::Call {
                                    callee: Box::new(Expr::Field {
                                        object: Box::new(Expr::Ident("checks".into(), zero())),
                                        field: "concat".into(),
                                        span: zero(),
                                    }),
                                    args: vec![Expr::Gen {
                                        body: Block {
                                            stmts: vec![Stmt::Expr(Expr::Int(0, zero()))],
                                            span: zero(),
                                        },
                                        span: zero(),
                                    }],
                                    span: zero(),
                                },
                                span: zero(),
                            },
                        ],
                        span: zero(),
                    },
                    span: zero(),
                },
                // return (Some { value: target.with_body(checks) }, Vec.new())
                Stmt::Return {
                    value: Some(Expr::Tuple(
                        vec![
                            Expr::StructLiteral {
                                ty: "Some".into(),
                                fields: vec![(
                                    "value".into(),
                                    Expr::Call {
                                        callee: Box::new(Expr::Field {
                                            object: Box::new(Expr::Ident("target".into(), zero())),
                                            field: "with_body".into(),
                                            span: zero(),
                                        }),
                                        args: vec![Expr::Ident("checks".into(), zero())],
                                        span: zero(),
                                    },
                                )],
                                span: zero(),
                            },
                            Expr::Call {
                                callee: Box::new(Expr::Field {
                                    object: Box::new(Expr::Ident("Vec".into(), zero())),
                                    field: "new".into(),
                                    span: zero(),
                                }),
                                args: vec![],
                                span: zero(),
                            },
                        ],
                        zero(),
                    )),
                    span: zero(),
                },
            ],
            span: zero(),
        };

        let proc = crate::parser::ast::ProcessorDef {
            annotation_name: "TestProc".into(),
            target_param: crate::parser::ast::Param {
                name: "target".into(),
                ty: TypeExpr::Named {
                    name: "FnDecl".into(),
                    generics: vec![],
                    bindings: vec![],
                    span: zero(),
                },
                span: zero(),
            },
            return_type: None,
            body: proc_body,
            span: zero(),
        };

        let result = Interpreter::run_processor(&fn_def, &proc, &[]).unwrap();
        let (replacement, extras) = result;
        assert!(extras.is_empty());
        if let Some(Item::Function(new_fn)) = replacement {
            assert_eq!(new_fn.body.stmts.len(), 3, "one stmt per param");
        } else {
            panic!("expected Some(Function)");
        }
    }
}
