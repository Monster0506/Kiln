use crate::diagnostics::Span;
use crate::parser::ast::{Block, Expr, FnDef, Item, ProcessorDef, Stmt, StringSegment, TypeExpr};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Runtime values for the processor interpreter
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
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
    FnDeclVal {
        fn_def: Box<FnDef>,
        annot_args: HashMap<String, Value>,
    },
    AnnotArgs(HashMap<String, Value>),
}

// ---------------------------------------------------------------------------
// Interpreter
// ---------------------------------------------------------------------------

pub struct Interpreter {
    env: HashMap<String, Value>,
}

enum StmtOutcome {
    Return(Value),
    Value(Value),
    Void,
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
        let target = Value::FnDeclVal {
            fn_def: Box::new(fn_def.clone()),
            annot_args: args_map,
        };
        interp.bind(&proc.target_param.name, target);

        let result = interp.eval_block(&proc.body).ok()?;
        decode_processor_result(result)
    }

    fn eval_block(&mut self, block: &Block) -> Result<Value, ()> {
        let mut last = Value::Void;
        for stmt in &block.stmts {
            match self.eval_stmt(stmt)? {
                StmtOutcome::Return(v) => return Ok(v),
                StmtOutcome::Value(v) => last = v,
                StmtOutcome::Void => {}
            }
        }
        Ok(last)
    }

    fn eval_stmt(&mut self, stmt: &Stmt) -> Result<StmtOutcome, ()> {
        match stmt {
            Stmt::Return { value: Some(e), .. } => Ok(StmtOutcome::Return(self.eval_expr(e)?)),
            Stmt::Return { value: None, .. } => Ok(StmtOutcome::Return(Value::Void)),
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
                let s: String = segs
                    .iter()
                    .filter_map(|seg| {
                        if let StringSegment::Text(t) = seg {
                            Some(t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");
                Ok(Value::Str(s))
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
            Stmt::Return {
                value: Some(e),
                span,
            } => Ok(Stmt::Return {
                value: Some(self.subst_expr(e)?),
                span: *span,
            }),
            Stmt::Expr(e) => Ok(Stmt::Expr(self.subst_expr(e)?)),
            Stmt::While { cond, body, span } => Ok(Stmt::While {
                cond: self.subst_expr(cond)?,
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
                let new_finally = finally.as_ref().map(|b| self.subst_block(b)).transpose()?;
                Ok(Stmt::TryCatch {
                    body: self.subst_block(body)?,
                    handlers: handlers.clone(),
                    finally: new_finally,
                    span: *span,
                })
            }
            _ => Ok(stmt.clone()),
        }
    }

    fn subst_block(&mut self, block: &Block) -> Result<Block, ()> {
        let stmts: Result<Vec<_>, _> = block.stmts.iter().map(|s| self.subst_stmt(s)).collect();
        Ok(Block {
            stmts: stmts?,
            span: block.span,
        })
    }

    fn subst_expr(&mut self, expr: &Expr) -> Result<Expr, ()> {
        match expr {
            Expr::GenSplice(inner, span) => {
                let val = self.eval_expr(inner)?;
                value_to_expr(val, *span)
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
            Expr::UnOp { op, operand, span } => Ok(Expr::UnOp {
                op: op.clone(),
                operand: Box::new(self.subst_expr(operand)?),
                span: *span,
            }),
            Expr::Tuple(elems, span) => {
                let new_elems: Result<Vec<_>, _> =
                    elems.iter().map(|e| self.subst_expr(e)).collect();
                Ok(Expr::Tuple(new_elems?, *span))
            }
            _ => Ok(expr.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers (no &mut self access needed)
// ---------------------------------------------------------------------------

fn eval_field(val: Value, field: &str) -> Result<Value, ()> {
    match val {
        Value::FnDeclVal {
            ref fn_def,
            ref annot_args,
        } => match field {
            "body" => Ok(Value::Block(fn_def.body.clone())),
            "name" => Ok(Value::Str(fn_def.name.clone())),
            "annot" => Ok(Value::AnnotArgs(annot_args.clone())),
            "return_type" => {
                let ty_str = type_expr_to_str(&fn_def.return_type);
                Ok(Value::Str(ty_str))
            }
            _ => Err(()),
        },
        Value::AnnotArgs(ref map) => map.get(field).cloned().ok_or(()),
        _ => Err(()),
    }
}

fn eval_static_method(type_name: &str, method: &str, args: &[Value]) -> Result<Value, ()> {
    match (type_name, method) {
        ("Vec", "new") if args.is_empty() => Ok(Value::List(vec![])),
        _ => Err(()),
    }
}

fn eval_method(obj: Value, method: &str, args: Vec<Value>) -> Result<Value, ()> {
    match (&obj, method) {
        (
            Value::FnDeclVal {
                fn_def,
                annot_args: _,
            },
            "with_body",
        ) => {
            if args.len() != 1 {
                return Err(());
            }
            if let Value::Block(new_body) = &args[0] {
                let mut new_fn = (**fn_def).clone();
                new_fn.body = new_body.clone();
                Ok(Value::Decl(Item::Function(new_fn)))
            } else {
                Err(())
            }
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
    fn gen_block_splice_of_block_extends_stmts() {
        let fn_def = simple_fn("foo", vec![void_stmt()]);
        let target = Value::FnDeclVal {
            fn_def: Box::new(fn_def),
            annot_args: HashMap::new(),
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
        let target = Value::FnDeclVal {
            fn_def: Box::new(fn_def),
            annot_args: HashMap::new(),
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
}
