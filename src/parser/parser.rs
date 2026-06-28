use super::ast::*;
use crate::diagnostics::Span;
use crate::lexer::{Token, TokenKind};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("lex error")]
    LexError,
    #[error("unexpected token {found:?}, expected {expected} at {span:?}")]
    Unexpected {
        found: TokenKind,
        expected: String,
        span: Span,
    },
    #[error("unexpected end of file, expected {expected}")]
    UnexpectedEof { expected: String },
}

impl ParseError {
    pub fn code(&self) -> &'static str {
        match self {
            ParseError::LexError => "P001",
            ParseError::Unexpected { .. } => "P002",
            ParseError::UnexpectedEof { .. } => "P003",
        }
    }

    pub fn kind(&self) -> &'static str {
        "syntax error"
    }

    pub fn message(&self) -> String {
        match self {
            ParseError::LexError => "lex error".into(),
            ParseError::Unexpected {
                found, expected, ..
            } => {
                format!("unexpected token {found:?}, expected {expected}")
            }
            ParseError::UnexpectedEof { expected } => {
                format!("unexpected end of file, expected {expected}")
            }
        }
    }

    pub fn span(&self) -> Option<Span> {
        match self {
            ParseError::LexError => None,
            ParseError::Unexpected { span, .. } => Some(*span),
            ParseError::UnexpectedEof { .. } => None,
        }
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(Span::new(0, 0))
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len().saturating_sub(1))];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.peek() == kind {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Span, ParseError> {
        if self.peek() == &kind {
            let span = self.peek_span();
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::Unexpected {
                found: self.peek().clone(),
                expected: format!("{kind:?}"),
                span: self.peek_span(),
            })
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(s) => {
                self.advance();
                Ok(s)
            }
            found => Err(ParseError::Unexpected {
                found,
                expected: "identifier".into(),
                span: self.peek_span(),
            }),
        }
    }

    /// Like `expect_ident` but also accepts reserved keywords in field-access position.
    /// Needed for `target.annotation.field`, `obj.type`, etc.
    fn expect_field_name(&mut self) -> Result<String, ParseError> {
        let span = self.peek_span();
        let name = match self.peek().clone() {
            TokenKind::Ident(s) => s,
            // Keywords that are valid field names in dot-access
            tok => match Self::keyword_as_str(&tok) {
                Some(s) => s.to_string(),
                None => {
                    return Err(ParseError::Unexpected {
                        found: tok,
                        expected: "field name".into(),
                        span,
                    })
                }
            },
        };
        self.advance();
        Ok(name)
    }

    fn keyword_as_str(tok: &TokenKind) -> Option<&'static str> {
        match tok {
            TokenKind::Annotation => Some("annotation"),
            TokenKind::Processor => Some("processor"),
            TokenKind::Struct => Some("struct"),
            TokenKind::Enum => Some("enum"),
            TokenKind::Interface => Some("interface"),
            TokenKind::Impl => Some("impl"),
            TokenKind::Def => Some("def"),
            TokenKind::Type => Some("type"),
            TokenKind::Return => Some("return"),
            TokenKind::Import => Some("import"),
            TokenKind::Export => Some("export"),
            _ => None,
        }
    }

    pub fn parse_file(&mut self) -> Result<SourceFile, ParseError> {
        let start = self.peek_span().start;
        let mut items = Vec::new();
        while self.peek() != &TokenKind::Eof {
            items.push(self.parse_item()?);
        }
        let end = self.peek_span().end;
        Ok(SourceFile {
            items,
            span: Span::new(start, end),
        })
    }

    fn parse_item(&mut self) -> Result<Item, ParseError> {
        let annotations = self.parse_annotation_uses()?;

        match self.peek().clone() {
            TokenKind::Def => Ok(Item::Function(self.parse_fn_def(annotations)?)),
            TokenKind::Struct => Ok(Item::Struct(self.parse_struct(annotations)?)),
            TokenKind::Enum => Ok(Item::Enum(self.parse_enum(annotations)?)),
            TokenKind::Interface => Ok(Item::Interface(self.parse_interface()?)),
            TokenKind::Impl => Ok(Item::ImplBlock(self.parse_impl(ImplKind::Plain)?)),
            TokenKind::Ident(ref s) if s == "specialized" || s == "extension" => {
                let kind = if s == "specialized" {
                    ImplKind::Specialized
                } else {
                    ImplKind::Extension
                };
                self.advance();
                self.expect(TokenKind::Impl)?;
                Ok(Item::ImplBlock(self.parse_impl_body(kind)?))
            }
            TokenKind::Annotation => Ok(Item::AnnotationDef(self.parse_annotation_def()?)),
            TokenKind::Processor => Ok(Item::ProcessorDef(self.parse_processor_def()?)),
            TokenKind::Type => Ok(Item::TypeAlias(self.parse_type_alias()?)),
            TokenKind::Import => Ok(Item::Import(self.parse_import()?)),
            TokenKind::Export => Ok(Item::Export(self.parse_export()?)),
            TokenKind::Const => Ok(Item::Const(self.parse_const_def()?)),
            TokenKind::Mut => Ok(Item::Global(self.parse_global(true)?)),
            TokenKind::Ident(_) if self.is_global_var_decl() => {
                Ok(Item::Global(self.parse_global(false)?))
            }
            found => Err(ParseError::Unexpected {
                found,
                expected: "item declaration".into(),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_annotation_uses(&mut self) -> Result<Vec<AnnotationUse>, ParseError> {
        let mut anns = Vec::new();
        while self.peek() == &TokenKind::At {
            let start = self.peek_span().start;
            self.advance();
            let name = self.expect_ident()?;
            let mut args = Vec::new();
            if self.eat(&TokenKind::LBrace) {
                while self.peek() != &TokenKind::RBrace {
                    let field = self.expect_ident()?;
                    self.expect(TokenKind::Colon)?;
                    let val = self.parse_expr(0)?;
                    args.push((field, val));
                    self.eat(&TokenKind::Comma);
                }
                self.expect(TokenKind::RBrace)?;
            } else if self.eat(&TokenKind::LParen) {
                while self.peek() != &TokenKind::RParen {
                    let ident = self.expect_ident()?;
                    let span = self.peek_span();
                    args.push((ident.clone(), Expr::Ident(ident, span)));
                    self.eat(&TokenKind::Comma);
                }
                self.expect(TokenKind::RParen)?;
            } else if self.eat(&TokenKind::LBracket) {
                while self.peek() != &TokenKind::RBracket {
                    let ident = self.expect_ident()?;
                    let span = self.peek_span();
                    args.push((ident.clone(), Expr::Ident(ident, span)));
                    self.eat(&TokenKind::Comma);
                }
                self.expect(TokenKind::RBracket)?;
            }
            let end = self.peek_span().start;
            anns.push(AnnotationUse {
                name,
                args,
                span: Span::new(start, end),
            });
        }
        Ok(anns)
    }

    /// Returns true when the current position looks like `ident: Type = expr` at item level
    /// (an immutable module-level variable).
    fn is_global_var_decl(&self) -> bool {
        // ident : ...
        matches!(self.tokens.get(self.pos), Some(t) if matches!(t.kind, TokenKind::Ident(_)))
            && matches!(self.tokens.get(self.pos + 1), Some(t) if t.kind == TokenKind::Colon)
    }

    fn parse_global(&mut self, mutable: bool) -> Result<GlobalVar, ParseError> {
        let start = self.peek_span().start;
        if mutable {
            self.expect(TokenKind::Mut)?;
        }
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        self.expect(TokenKind::Eq)?;
        let value = self.parse_expr(0)?;
        let end = self.peek_span().start;
        Ok(GlobalVar {
            name,
            ty,
            value,
            mutable,
            span: Span::new(start, end),
        })
    }

    fn parse_const_def(&mut self) -> Result<crate::parser::ast::ConstDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Const)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        self.expect(TokenKind::Eq)?;
        let value = self.parse_expr(0)?;
        let end = self.peek_span().start;
        Ok(crate::parser::ast::ConstDef {
            name,
            ty,
            value,
            span: Span::new(start, end),
        })
    }

    fn parse_import(&mut self) -> Result<Import, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Import)?;
        let mut path = vec![self.expect_ident()?];
        while self.eat(&TokenKind::Dot) {
            path.push(self.expect_ident()?);
        }
        self.expect(TokenKind::LBrace)?;
        let mut symbols = Vec::new();
        while self.peek() != &TokenKind::RBrace {
            if self.eat(&TokenKind::Star) {
                symbols.push("*".to_string());
            } else {
                symbols.push(self.expect_ident()?);
            }
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(Import {
            path,
            symbols,
            span: Span::new(start, self.peek_span().start),
        })
    }

    fn parse_export(&mut self) -> Result<Export, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Export)?;
        self.expect(TokenKind::LBrace)?;
        let mut symbols = Vec::new();
        while self.peek() != &TokenKind::RBrace {
            if self.eat(&TokenKind::Star) {
                symbols.push("*".to_string());
            } else {
                symbols.push(self.expect_ident()?);
            }
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(Export {
            symbols,
            span: Span::new(start, self.peek_span().start),
        })
    }

    fn parse_type(&mut self) -> Result<TypeExpr, ParseError> {
        let base = self.parse_type_atom()?;

        if self.peek() == &TokenKind::Pipe {
            let start = base.span();
            let mut variants = vec![base];
            while self.eat(&TokenKind::Pipe) {
                variants.push(self.parse_type_atom()?);
            }
            let end = variants.last().unwrap().span();
            return Ok(TypeExpr::Union(variants, Span::new(start.start, end.end)));
        }

        if self.peek() == &TokenKind::Plus {
            let start = base.span();
            let mut parts = vec![base];
            while self.eat(&TokenKind::Plus) {
                parts.push(self.parse_type_atom()?);
            }
            let end = parts.last().unwrap().span();
            return Ok(TypeExpr::Compound(parts, Span::new(start.start, end.end)));
        }

        Ok(base)
    }

    fn parse_type_atom(&mut self) -> Result<TypeExpr, ParseError> {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::LParen => {
                self.advance();
                let mut types = Vec::new();
                while self.peek() != &TokenKind::RParen {
                    types.push(self.parse_type()?);
                    self.eat(&TokenKind::Comma);
                }
                self.expect(TokenKind::RParen)?;
                let end = self.peek_span();
                if types.len() == 1 {
                    return Ok(types.remove(0));
                }
                Ok(TypeExpr::Tuple(types, Span::new(start.start, end.start)))
            }
            TokenKind::Amp => {
                self.advance();
                // Lifetime `&a T` / `&static T`: present when next token is an ident
                // followed by another ident or `mut`.
                let lifetime = if let TokenKind::Ident(_) = self.peek() {
                    let next_is_type_or_mut = matches!(
                        self.tokens.get(self.pos + 1).map(|t| &t.kind),
                        Some(TokenKind::Ident(_)) | Some(TokenKind::Mut)
                    );
                    if next_is_type_or_mut {
                        Some(self.expect_ident()?)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mutable = self.eat(&TokenKind::Mut);
                let inner = self.parse_type_atom()?;
                let end = inner.span();
                Ok(TypeExpr::Ref {
                    mutable,
                    lifetime,
                    inner: Box::new(inner),
                    span: Span::new(start.start, end.end),
                })
            }
            TokenKind::Ident(_) | TokenKind::Self_ => {
                let name = match self.peek().clone() {
                    TokenKind::Self_ => {
                        self.advance();
                        "Self".to_string()
                    }
                    TokenKind::Ident(s) => {
                        self.advance();
                        s
                    }
                    _ => unreachable!(),
                };

                if name == "Callable" && self.peek() == &TokenKind::LBracket {
                    self.advance();
                    self.expect(TokenKind::LParen)?;
                    let mut params = Vec::new();
                    while self.peek() != &TokenKind::RParen {
                        params.push(self.parse_type()?);
                        self.eat(&TokenKind::Comma);
                    }
                    self.expect(TokenKind::RParen)?;
                    self.expect(TokenKind::Comma)?;
                    let ret = self.parse_type()?;
                    self.expect(TokenKind::RBracket)?;
                    let end = self.peek_span();
                    return Ok(TypeExpr::Callable {
                        params,
                        ret: Box::new(ret),
                        span: Span::new(start.start, end.start),
                    });
                }

                // Check for associated type projection: `Base.Assoc`
                if self.eat(&TokenKind::Dot) {
                    let assoc = self.expect_ident()?;
                    let end = self.peek_span();
                    return Ok(TypeExpr::Projection {
                        base: name,
                        assoc,
                        span: Span::new(start.start, end.start),
                    });
                }

                let mut generics = Vec::new();
                let mut bindings = Vec::new();
                if self.eat(&TokenKind::LBracket) {
                    while self.peek() != &TokenKind::RBracket {
                        // Peek ahead: `Ident Eq` means a named binding like `Output=Self`
                        if matches!(self.peek(), TokenKind::Ident(_)) {
                            let saved_pos = self.pos;
                            let binding_name = self.expect_ident()?;
                            if self.eat(&TokenKind::Eq) {
                                let binding_ty = self.parse_type()?;
                                bindings.push((binding_name, binding_ty));
                                self.eat(&TokenKind::Comma);
                                continue;
                            } else {
                                // Not a binding -- backtrack and parse as a generic type
                                self.pos = saved_pos;
                            }
                        }
                        generics.push(self.parse_type()?);
                        self.eat(&TokenKind::Comma);
                    }
                    self.expect(TokenKind::RBracket)?;
                }
                let end = self.peek_span();
                Ok(TypeExpr::Named {
                    name,
                    generics,
                    bindings,
                    span: Span::new(start.start, end.start),
                })
            }
            TokenKind::Void => {
                self.advance();
                Ok(TypeExpr::Named {
                    name: "void".to_string(),
                    generics: vec![],
                    bindings: vec![],
                    span: start,
                })
            }
            TokenKind::LtLt => {
                self.advance();
                let expr = self.parse_expr(0)?;
                self.expect(TokenKind::GtGt)?;
                let end = self.peek_span().start;
                Ok(TypeExpr::GenSplice(
                    Box::new(expr),
                    Span::new(start.start, end),
                ))
            }
            found => Err(ParseError::Unexpected {
                found,
                expected: "type expression".into(),
                span: start,
            }),
        }
    }

    fn parse_generic_params(&mut self) -> Result<Vec<GenericParam>, ParseError> {
        if !self.eat(&TokenKind::LBracket) {
            return Ok(vec![]);
        }
        let mut params = Vec::new();
        while self.peek() != &TokenKind::RBracket {
            let start = self.peek_span();
            if self.eat(&TokenKind::Scope) {
                let name = self.expect_ident()?;
                // Lifetime params support at most one bound (outlives relation).
                let bounds = if self.eat(&TokenKind::Colon) {
                    vec![self.parse_type_atom()?]
                } else {
                    vec![]
                };
                let end = self.peek_span();
                params.push(GenericParam {
                    kind: GenericParamKind::Lifetime,
                    variance: Variance::Invariant,
                    name,
                    bounds,
                    span: Span::new(start.start, end.start),
                });
            } else {
                // Optional variance annotation: `+T` (covariant) or `-T` (contravariant).
                let variance = if self.eat(&TokenKind::Plus) {
                    Variance::Covariant
                } else if self.eat(&TokenKind::Minus) {
                    Variance::Contravariant
                } else {
                    Variance::Invariant
                };
                let name = self.expect_ident()?;
                // Check for higher-kinded type constructor syntax: F[_]
                let param_kind = if self.peek() == &TokenKind::LBracket {
                    self.advance();
                    // Consume the wildcard `_` (or any ident)
                    self.expect_ident()?;
                    self.expect(TokenKind::RBracket)?;
                    GenericParamKind::TypeConstructor
                } else {
                    GenericParamKind::Type
                };
                // Generic param bounds: `T:Bound`, `T:A+B`, or `T:(A[X=Y], B)`.
                let bounds = if self.eat(&TokenKind::Colon) {
                    if self.eat(&TokenKind::LParen) {
                        let mut bs = Vec::new();
                        while self.peek() != &TokenKind::RParen {
                            bs.push(self.parse_type_atom()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                        self.expect(TokenKind::RParen)?;
                        bs
                    } else {
                        let mut bs = vec![self.parse_type_atom()?];
                        while self.eat(&TokenKind::Plus) {
                            bs.push(self.parse_type_atom()?);
                        }
                        bs
                    }
                } else {
                    vec![]
                };
                let end = self.peek_span();
                params.push(GenericParam {
                    kind: param_kind,
                    variance,
                    name,
                    bounds,
                    span: Span::new(start.start, end.start),
                });
            }
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBracket)?;
        Ok(params)
    }

    fn parse_fn_def(&mut self, annotations: Vec<AnnotationUse>) -> Result<FnDef, ParseError> {
        let start = self.peek_span().start;
        let _is_builtin = annotations.iter().any(|a| a.name == "builtin");
        self.expect(TokenKind::Def)?;
        let name = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        let mut variadic = None;
        while self.peek() != &TokenKind::RParen {
            let ps = self.peek_span();
            if self.eat(&TokenKind::Star) {
                let vname = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                variadic = Some(VariadicParam {
                    name: vname,
                    ty,
                    span: ps,
                });
                break;
            }
            let mutable = self.eat(&TokenKind::Mut);
            let pname = self.expect_ident()?;
            // `self` with no type annotation gets implicit type `Self`
            let ty = if self.peek() == &TokenKind::Colon {
                self.advance();
                self.parse_type()?
            } else if pname == "self" {
                let sp = ps;
                TypeExpr::Named {
                    name: "Self".to_string(),
                    generics: vec![],
                    bindings: vec![],
                    span: sp,
                }
            } else {
                return Err(ParseError::Unexpected {
                    found: self.peek().clone(),
                    expected: "Colon".into(),
                    span: self.peek_span(),
                });
            };
            let end = self.peek_span();
            params.push(Param {
                name: pname,
                ty,
                mutable,
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        let return_type = if self.eat(&TokenKind::Arrow) {
            self.parse_type()?
        } else {
            let sp = self.peek_span();
            TypeExpr::Named {
                name: "void".to_string(),
                generics: vec![],
                bindings: vec![],
                span: sp,
            }
        };
        let body_span = self.peek_span();
        let (body, is_declaration) = if self.peek() != &TokenKind::LBrace {
            (
                Block {
                    stmts: vec![],
                    span: body_span,
                },
                true,
            )
        } else {
            (self.parse_block()?, false)
        };
        let end = self.peek_span().start;
        Ok(FnDef {
            annotations,
            name,
            generic_params,
            params,
            variadic,
            return_type,
            body,
            is_declaration,
            span: Span::new(start, end),
        })
    }

    fn parse_interfaces(&mut self) -> Result<Vec<TypeExpr>, ParseError> {
        if !self.eat(&TokenKind::Colon) {
            return Ok(vec![]);
        }
        let mut ifaces = vec![self.parse_type_atom()?];
        while self.eat(&TokenKind::Comma) {
            ifaces.push(self.parse_type_atom()?);
        }
        Ok(ifaces)
    }

    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let start = self.peek_span().start;
        let annotations = self.parse_annotation_uses()?;
        let is_priv = self.eat(&TokenKind::Priv);
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        let default = if self.eat(&TokenKind::Eq) {
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        let end = self.peek_span().start;
        Ok(Field {
            annotations,
            is_priv,
            name,
            ty,
            default,
            span: Span::new(start, end),
        })
    }

    fn parse_struct(&mut self, annotations: Vec<AnnotationUse>) -> Result<StructDef, ParseError> {
        let is_builtin = annotations.iter().any(|a| a.name == "builtin");
        let start = self.peek_span().start;
        self.expect(TokenKind::Struct)?;
        let name = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        let interfaces = self.parse_interfaces()?;
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut decls = Vec::new();
        let mut inline_hooks = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            if self.peek() == &TokenKind::Def || self.is_annotated_def() {
                let anns = self.parse_annotation_uses()?;
                if is_builtin && self.is_bodyless_def() {
                    decls.push(self.parse_fn_decl()?);
                } else {
                    methods.push(self.parse_fn_def(anns)?);
                }
            } else if self.peek() == &TokenKind::Hook || self.is_annotated_hook() {
                let anns = self.parse_annotation_uses()?;
                inline_hooks.push(self.parse_hook_def_with_annotations(anns)?);
            } else {
                fields.push(self.parse_field()?);
                self.eat(&TokenKind::Comma);
            }
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(StructDef {
            annotations,
            is_builtin,
            name,
            generic_params,
            interfaces,
            fields,
            methods,
            decls,
            inline_hooks,
            span: Span::new(start, end),
        })
    }

    /// Returns true when the next `def` has no `{` body (bodyless declaration).
    fn is_bodyless_def(&self) -> bool {
        let mut i = self.pos;
        // skip `def`
        if i < self.tokens.len() && self.tokens[i].kind == TokenKind::Def {
            i += 1;
        } else {
            return false;
        }
        // skip name
        if i < self.tokens.len() {
            i += 1;
        }
        // skip optional generic params [...]
        if i < self.tokens.len() && self.tokens[i].kind == TokenKind::LBracket {
            let mut depth = 1;
            i += 1;
            while i < self.tokens.len() && depth > 0 {
                match self.tokens[i].kind {
                    TokenKind::LBracket => depth += 1,
                    TokenKind::RBracket => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
        }
        // skip param list (...)
        if i < self.tokens.len() && self.tokens[i].kind == TokenKind::LParen {
            let mut depth = 1;
            i += 1;
            while i < self.tokens.len() && depth > 0 {
                match self.tokens[i].kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
        }
        // skip -> ReturnType
        if i < self.tokens.len() && self.tokens[i].kind == TokenKind::Arrow {
            i += 1;
            // skip type tokens until we hit `{`, `def`, `}`, or EOF
            while i < self.tokens.len() {
                match self.tokens[i].kind {
                    TokenKind::LBrace | TokenKind::RBrace | TokenKind::Def | TokenKind::Eof => {
                        break;
                    }
                    _ => i += 1,
                }
            }
        }
        // bodyless if the next token is NOT `{`
        i < self.tokens.len() && self.tokens[i].kind != TokenKind::LBrace
    }

    fn parse_fn_decl(&mut self) -> Result<FnDecl, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Def)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while self.peek() != &TokenKind::RParen && self.peek() != &TokenKind::Eof {
            let ps = self.peek_span();
            let mutable = self.eat(&TokenKind::Mut);
            let pname = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let end = self.peek_span();
            params.push(Param {
                name: pname,
                ty,
                mutable,
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let return_type = self.parse_type()?;
        let end = self.peek_span().start;
        Ok(FnDecl {
            name,
            params,
            return_type,
            span: Span::new(start, end),
        })
    }

    fn is_annotated_def(&self) -> bool {
        let mut i = self.pos;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::At => {
                    i += 1;
                    if i < self.tokens.len() {
                        i += 1;
                    }
                }
                TokenKind::Def => return true,
                _ => return false,
            }
        }
        false
    }

    /// Returns true when the current position starts an annotated `hook` keyword,
    /// skipping over `@name`, `@name[...]`, `@name(...)`, or `@name{...}` sequences.
    fn is_annotated_hook(&self) -> bool {
        let mut i = self.pos;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::At => {
                    i += 1;
                    if i >= self.tokens.len() {
                        return false;
                    }
                    i += 1; // skip annotation name
                            // skip optional argument list
                    if i < self.tokens.len() {
                        let (open, close) = match &self.tokens[i].kind {
                            TokenKind::LBracket => (TokenKind::LBracket, TokenKind::RBracket),
                            TokenKind::LParen => (TokenKind::LParen, TokenKind::RParen),
                            TokenKind::LBrace => (TokenKind::LBrace, TokenKind::RBrace),
                            _ => continue,
                        };
                        let mut depth = 1usize;
                        i += 1;
                        while i < self.tokens.len() && depth > 0 {
                            if self.tokens[i].kind == open {
                                depth += 1;
                            } else if self.tokens[i].kind == close {
                                depth -= 1;
                            }
                            i += 1;
                        }
                    }
                }
                TokenKind::Hook => return true,
                _ => return false,
            }
        }
        false
    }

    fn parse_enum_variant(&mut self) -> Result<EnumVariant, ParseError> {
        let start = self.peek_span().start;
        let name = self.expect_ident()?;
        let mut fields = Vec::new();
        let mut discriminant = None;
        if self.eat(&TokenKind::LBrace) {
            while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
                fields.push(self.parse_field()?);
                self.eat(&TokenKind::Comma);
            }
            self.expect(TokenKind::RBrace)?;
        } else if self.eat(&TokenKind::Eq) {
            match self.peek().clone() {
                TokenKind::Int(n) => {
                    self.advance();
                    discriminant = Some(n);
                }
                found => {
                    return Err(ParseError::Unexpected {
                        found,
                        expected: "integer discriminant".into(),
                        span: self.peek_span(),
                    })
                }
            }
        }
        let end = self.peek_span().start;
        Ok(EnumVariant {
            name,
            fields,
            discriminant,
            span: Span::new(start, end),
        })
    }

    fn parse_enum(&mut self, annotations: Vec<AnnotationUse>) -> Result<EnumDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Enum)?;
        let name = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        let interfaces = self.parse_interfaces()?;
        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();
        let mut methods = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            if self.peek() == &TokenKind::Def || self.is_annotated_def() {
                let anns = self.parse_annotation_uses()?;
                methods.push(self.parse_fn_def(anns)?);
            } else {
                variants.push(self.parse_enum_variant()?);
                self.eat(&TokenKind::Comma);
            }
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(EnumDef {
            annotations,
            name,
            generic_params,
            interfaces,
            variants,
            methods,
            span: Span::new(start, end),
        })
    }

    fn parse_hook_name(&mut self) -> Result<HookName, ParseError> {
        match self.peek().clone() {
            TokenKind::Plus => {
                self.advance();
                Ok(HookName::Op("+".into()))
            }
            TokenKind::PlusEq => {
                self.advance();
                Ok(HookName::Op("+=".into()))
            }
            TokenKind::Minus => {
                self.advance();
                Ok(HookName::Op("-".into()))
            }
            TokenKind::MinusEq => {
                self.advance();
                Ok(HookName::Op("-=".into()))
            }
            TokenKind::Star => {
                self.advance();
                Ok(HookName::Op("*".into()))
            }
            TokenKind::StarEq => {
                self.advance();
                Ok(HookName::Op("*=".into()))
            }
            TokenKind::Slash => {
                self.advance();
                Ok(HookName::Op("/".into()))
            }
            TokenKind::SlashEq => {
                self.advance();
                Ok(HookName::Op("/=".into()))
            }
            TokenKind::Percent => {
                self.advance();
                Ok(HookName::Op("%".into()))
            }
            TokenKind::PercentEq => {
                self.advance();
                Ok(HookName::Op("%=".into()))
            }
            TokenKind::Bang => {
                self.advance();
                Ok(HookName::Op("!".into()))
            }
            TokenKind::EqEq => {
                self.advance();
                Ok(HookName::Op("==".into()))
            }
            TokenKind::Spaceship => {
                self.advance();
                Ok(HookName::Op("<=>".into()))
            }
            TokenKind::Lt => {
                self.advance();
                Ok(HookName::Op("<".into()))
            }
            TokenKind::Gt => {
                self.advance();
                Ok(HookName::Op(">".into()))
            }
            TokenKind::LtEq => {
                self.advance();
                Ok(HookName::Op("<=".into()))
            }
            TokenKind::GtEq => {
                self.advance();
                Ok(HookName::Op(">=".into()))
            }
            TokenKind::LBracket => {
                self.advance();
                self.expect(TokenKind::RBracket)?;
                if self.eat(&TokenKind::Eq) {
                    Ok(HookName::Op("[]=".into()))
                } else {
                    Ok(HookName::Op("[]".into()))
                }
            }
            TokenKind::LParen => {
                self.advance();
                self.expect(TokenKind::RParen)?;
                Ok(HookName::Op("()".into()))
            }
            TokenKind::Amp => {
                self.advance();
                Ok(HookName::Op("&".into()))
            }
            TokenKind::Pipe => {
                self.advance();
                Ok(HookName::Op("|".into()))
            }
            TokenKind::Caret => {
                self.advance();
                Ok(HookName::Op("^".into()))
            }
            TokenKind::Ident(name) => {
                self.advance();
                Ok(HookName::Named(name))
            }
            found => Err(ParseError::Unexpected {
                found,
                expected: "hook name".into(),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_hook_sig_or_def_with_annotations(
        &mut self,
        annotations: Vec<AnnotationUse>,
    ) -> Result<InterfaceItem, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Hook)?;
        let name = self.parse_hook_name()?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while self.peek() != &TokenKind::RParen {
            let ps = self.peek_span();
            let mutable = self.eat(&TokenKind::Mut);
            let pname = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let end = self.peek_span();
            params.push(Param {
                name: pname,
                ty,
                mutable,
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        let return_type = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let default = if self.peek() == &TokenKind::LBrace {
            Some(self.parse_block()?)
        } else {
            None
        };
        let end = self.peek_span().start;
        Ok(InterfaceItem {
            kind: InterfaceItemKind::Hook {
                annotations,
                name,
                params,
                return_type,
                default,
            },
            span: Span::new(start, end),
        })
    }

    fn parse_interface(&mut self) -> Result<InterfaceDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Interface)?;
        let name = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        let extends = self.parse_interfaces()?;
        self.expect(TokenKind::LBrace)?;
        let mut items = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            let item_start = self.peek_span().start;
            let anns = self.parse_annotation_uses()?;
            let item = match self.peek().clone() {
                TokenKind::Hook => self.parse_hook_sig_or_def_with_annotations(anns)?,
                TokenKind::Def => {
                    let method = self.parse_fn_def(anns)?;
                    let span = method.span;
                    InterfaceItem {
                        kind: InterfaceItemKind::Method(method),
                        span,
                    }
                }
                TokenKind::Type => {
                    self.advance();
                    let assoc_name = self.expect_ident()?;
                    let bounds = if self.eat(&TokenKind::Colon) {
                        let mut bs = vec![self.expect_ident()?];
                        while self.eat(&TokenKind::Comma) {
                            if let TokenKind::Ident(_) = self.peek() {
                                bs.push(self.expect_ident()?);
                            } else {
                                break;
                            }
                        }
                        bs
                    } else {
                        vec![]
                    };
                    let end = self.peek_span().start;
                    InterfaceItem {
                        kind: InterfaceItemKind::AssocType {
                            name: assoc_name,
                            bounds,
                        },
                        span: Span::new(item_start, end),
                    }
                }
                TokenKind::Ident(_) => {
                    let field_name = self.expect_ident()?;
                    self.expect(TokenKind::Colon)?;
                    let ty = self.parse_type()?;
                    let end = self.peek_span().start;
                    InterfaceItem {
                        kind: InterfaceItemKind::Field {
                            name: field_name,
                            ty,
                        },
                        span: Span::new(item_start, end),
                    }
                }
                found => {
                    return Err(ParseError::Unexpected {
                        found,
                        expected: "interface item".into(),
                        span: self.peek_span(),
                    })
                }
            };
            items.push(item);
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(InterfaceDef {
            name,
            generic_params,
            extends,
            items,
            span: Span::new(start, end),
        })
    }

    fn parse_hook_def_with_annotations(
        &mut self,
        annotations: Vec<AnnotationUse>,
    ) -> Result<HookDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Hook)?;
        let name = self.parse_hook_name()?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while self.peek() != &TokenKind::RParen {
            let ps = self.peek_span();
            let mutable = self.eat(&TokenKind::Mut);
            let pname = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let end = self.peek_span();
            params.push(Param {
                name: pname,
                ty,
                mutable,
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        let return_type = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(HookDef {
            annotations,
            name,
            params,
            return_type,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_impl(&mut self, kind: ImplKind) -> Result<ImplBlock, ParseError> {
        self.expect(TokenKind::Impl)?;
        self.parse_impl_body(kind)
    }

    fn parse_impl_body(&mut self, kind: ImplKind) -> Result<ImplBlock, ParseError> {
        let start = self.peek_span().start;
        let generic_params = self.parse_generic_params()?;
        let interface = self.parse_type_atom()?;
        self.expect(TokenKind::For)?;
        let for_type = self.parse_type_atom()?;
        let self_alias = if self.eat(&TokenKind::LParen) {
            let alias = self.expect_ident()?;
            self.expect(TokenKind::RParen)?;
            Some(alias)
        } else {
            None
        };
        self.expect(TokenKind::LBrace)?;
        let mut methods = Vec::new();
        let mut hooks = Vec::new();
        let mut assoc_bindings = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            let anns = self.parse_annotation_uses()?;
            match self.peek().clone() {
                TokenKind::Hook => hooks.push(self.parse_hook_def_with_annotations(anns)?),
                TokenKind::Def => methods.push(self.parse_fn_def(anns)?),
                TokenKind::Type => {
                    self.advance();
                    let binding_name = self.expect_ident()?;
                    self.expect(TokenKind::Eq)?;
                    let binding_ty = self.parse_type()?;
                    assoc_bindings.push((binding_name, binding_ty));
                }
                found => {
                    return Err(ParseError::Unexpected {
                        found,
                        expected: "hook, def, or type binding in impl block".into(),
                        span: self.peek_span(),
                    })
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(ImplBlock {
            generic_params,
            interface,
            for_type,
            self_alias,
            methods,
            hooks,
            assoc_bindings,
            kind,
            span: Span::new(start, end),
        })
    }

    fn parse_annotation_def(&mut self) -> Result<AnnotationDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Annotation)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            fields.push(self.parse_field()?);
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(AnnotationDef {
            name,
            fields,
            span: Span::new(start, end),
        })
    }

    fn parse_processor_def(&mut self) -> Result<ProcessorDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Processor)?;
        let annotation_name = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;
        let ps = self.peek_span();
        let pname = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        let pe = self.peek_span();
        let target_param = Param {
            name: pname,
            ty,
            mutable: false,
            span: Span::new(ps.start, pe.start),
        };
        self.expect(TokenKind::RParen)?;
        let return_type = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(ProcessorDef {
            annotation_name,
            target_param,
            return_type,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_type_alias(&mut self) -> Result<TypeAlias, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Type)?;
        let name = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        self.expect(TokenKind::Eq)?;
        let ty = self.parse_type()?;
        let end = self.peek_span().start;
        Ok(TypeAlias {
            name,
            generic_params,
            ty,
            span: Span::new(start, end),
        })
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(Block {
            stmts,
            span: Span::new(start, end),
        })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Return => {
                self.advance();
                let value = if self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
                    Some(self.parse_expr(0)?)
                } else {
                    None
                };
                Ok(Stmt::Return { value, span: start })
            }
            TokenKind::Raise => {
                self.advance();
                let value = if self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
                    Some(self.parse_expr(0)?)
                } else {
                    None
                };
                Ok(Stmt::Raise { value, span: start })
            }
            TokenKind::Break => {
                self.advance();
                Ok(Stmt::Break(start))
            }
            TokenKind::Continue => {
                self.advance();
                Ok(Stmt::Continue(start))
            }
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::Do => self.parse_do_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Try => self.parse_try(),
            TokenKind::Def => Ok(Stmt::FnDef(self.parse_fn_def(vec![])?)),
            TokenKind::At => {
                let anns = self.parse_annotation_uses()?;
                Ok(Stmt::FnDef(self.parse_fn_def(anns)?))
            }
            TokenKind::Mut => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                self.expect(TokenKind::Eq)?;
                let value = self.parse_expr(0)?;
                let end = self.peek_span().start;
                Ok(Stmt::VarDecl {
                    name,
                    ty,
                    value,
                    mutable: true,
                    span: Span::new(start.start, end),
                })
            }
            TokenKind::Ident(_) => self.parse_ident_led_stmt(),
            _ => {
                let expr = self.parse_expr(0)?;
                if self.eat(&TokenKind::Eq) {
                    let value = self.parse_expr(0)?;
                    let end = self.peek_span().start;
                    Ok(Stmt::Assign {
                        target: expr,
                        value,
                        span: Span::new(start.start, end),
                    })
                } else if let Some(bin_op) = compound_assign_op(self.peek()) {
                    self.advance();
                    let rhs = self.parse_expr(0)?;
                    let end = self.peek_span().start;
                    Ok(Stmt::CompoundAssign {
                        target: expr,
                        op: bin_op,
                        rhs,
                        span: Span::new(start.start, end),
                    })
                } else {
                    Ok(Stmt::Expr(expr))
                }
            }
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::If)?;
        let cond = self.parse_expr_no_struct(0)?;
        let body = self.parse_block()?;
        let mut branches = vec![(cond, body)];
        while self.eat(&TokenKind::Elif) {
            let c = self.parse_expr_no_struct(0)?;
            let b = self.parse_block()?;
            branches.push((c, b));
        }
        let else_branch = if self.eat(&TokenKind::Else) {
            Some(self.parse_block()?)
        } else {
            None
        };
        let end = self.peek_span().start;
        Ok(Stmt::If {
            branches,
            else_branch,
            span: Span::new(start, end),
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::While)?;
        let cond = self.parse_expr_no_struct(0)?;
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(Stmt::While {
            cond,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_do_while(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Do)?;
        let body = self.parse_block()?;
        self.expect(TokenKind::While)?;
        let cond = self.parse_expr_no_struct(0)?;
        let end = self.peek_span().start;
        Ok(Stmt::DoWhile {
            body,
            cond,
            span: Span::new(start, end),
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::For)?;
        let binding = self.expect_ident()?;
        let binding_ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(TokenKind::LArrow)?;
        let iterable = self.parse_expr_no_struct(0)?;
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(Stmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Try)?;
        let body = self.parse_block()?;
        let mut handlers = Vec::new();
        while self.eat(&TokenKind::Except) {
            let hs = self.peek_span().start;
            let ty = self.parse_type()?;
            self.expect(TokenKind::As)?;
            let binding = self.expect_ident()?;
            let hbody = self.parse_block()?;
            let he = self.peek_span().start;
            handlers.push(CatchHandler {
                ty,
                binding,
                body: hbody,
                span: Span::new(hs, he),
            });
        }
        let finally = if self.eat(&TokenKind::Finally) {
            Some(self.parse_block()?)
        } else {
            None
        };
        let end = self.peek_span().start;
        Ok(Stmt::TryCatch {
            body,
            handlers,
            finally,
            span: Span::new(start, end),
        })
    }

    fn parse_ident_led_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        let name = self.expect_ident()?;

        if self.eat(&TokenKind::Colon) {
            let ty = self.parse_type()?;
            self.expect(TokenKind::Eq)?;
            let value = self.parse_expr(0)?;
            let end = self.peek_span().start;
            return Ok(Stmt::VarDecl {
                name,
                ty,
                value,
                mutable: false,
                span: Span::new(start.start, end),
            });
        }

        let ident_expr = Expr::Ident(name, start);
        let lhs = self.parse_postfix_chain(ident_expr)?;

        if self.eat(&TokenKind::Eq) {
            let value = self.parse_expr(0)?;
            let end = self.peek_span().start;
            return Ok(Stmt::Assign {
                target: lhs,
                value,
                span: Span::new(start.start, end),
            });
        }

        if let Some(bin_op) = compound_assign_op(self.peek()) {
            self.advance();
            let rhs = self.parse_expr(0)?;
            let end = self.peek_span().start;
            return Ok(Stmt::CompoundAssign {
                target: lhs,
                op: bin_op,
                rhs,
                span: Span::new(start.start, end),
            });
        }

        let expr = self.parse_infix(lhs, 0, true)?;
        Ok(Stmt::Expr(expr))
    }

    fn infix_bp(op: &TokenKind) -> Option<(u8, u8)> {
        match op {
            TokenKind::PipePipe => Some((1, 2)),
            TokenKind::AmpAmp => Some((3, 4)),
            TokenKind::EqEq | TokenKind::BangEq => Some((5, 6)),
            TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::LtEq
            | TokenKind::GtEq
            | TokenKind::Spaceship => Some((7, 8)),
            TokenKind::Plus | TokenKind::Minus => Some((9, 10)),
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Some((11, 12)),
            _ => None,
        }
    }

    fn postfix_bp(op: &TokenKind) -> Option<u8> {
        match op {
            TokenKind::Dot
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::Question
            | TokenKind::As => Some(13),
            _ => None,
        }
    }

    pub fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        self.parse_expr_inner(min_bp, true)
    }

    fn parse_expr_no_struct(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        self.parse_expr_inner(min_bp, false)
    }

    fn parse_expr_inner(&mut self, min_bp: u8, allow_struct: bool) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_primary(allow_struct)?;
        lhs = self.parse_postfix_chain(lhs)?;
        self.parse_infix(lhs, min_bp, allow_struct)
    }

    fn parse_postfix_chain(&mut self, mut lhs: Expr) -> Result<Expr, ParseError> {
        loop {
            if Self::postfix_bp(self.peek()).is_some() {
                lhs = self.parse_postfix(lhs)?;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_infix(
        &mut self,
        mut lhs: Expr,
        min_bp: u8,
        allow_struct: bool,
    ) -> Result<Expr, ParseError> {
        loop {
            if let Some(lbp) = Self::postfix_bp(self.peek()) {
                if lbp < min_bp {
                    break;
                }
                lhs = self.parse_postfix(lhs)?;
                continue;
            }
            if let Some((lbp, rbp)) = Self::infix_bp(self.peek()) {
                if lbp < min_bp {
                    break;
                }
                let op_kind = self.peek().clone();
                let op_span = self.peek_span();
                self.advance();
                let rhs = self.parse_expr_inner(rbp, allow_struct)?;
                let span = Span::new(lhs.span().start, rhs.span().end);
                lhs = Expr::BinOp {
                    op: Self::token_to_binop(&op_kind),
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                    span,
                };
                let _ = op_span;
                continue;
            }
            break;
        }
        Ok(lhs)
    }

    fn token_to_binop(kind: &TokenKind) -> BinOp {
        match kind {
            TokenKind::Plus => BinOp::Add,
            TokenKind::Minus => BinOp::Sub,
            TokenKind::Star => BinOp::Mul,
            TokenKind::Slash => BinOp::Div,
            TokenKind::Percent => BinOp::Mod,
            TokenKind::EqEq => BinOp::Eq,
            TokenKind::BangEq => BinOp::Ne,
            TokenKind::Lt => BinOp::Lt,
            TokenKind::Gt => BinOp::Gt,
            TokenKind::LtEq => BinOp::LtEq,
            TokenKind::GtEq => BinOp::GtEq,
            TokenKind::Spaceship => BinOp::Spaceship,
            TokenKind::AmpAmp => BinOp::And,
            TokenKind::PipePipe => BinOp::Or,
            TokenKind::Pipe => BinOp::Pipe,
            _ => unreachable!("not a binop: {kind:?}"),
        }
    }

    fn parse_primary(&mut self, allow_struct: bool) -> Result<Expr, ParseError> {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Int(n) => {
                self.advance();
                Ok(Expr::Int(n, start))
            }
            TokenKind::Float(f) => {
                self.advance();
                Ok(Expr::Float(f, start))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Bool(true, start))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Bool(false, start))
            }
            TokenKind::StringStart => self.parse_string_expr(),
            TokenKind::LParen => self.parse_closure_or_paren(),
            TokenKind::Ident(_) => {
                let name = self.expect_ident()?;
                // `EnumName:Variant` enum access
                if self.peek() == &TokenKind::Colon {
                    if let TokenKind::Ident(_) = self
                        .tokens
                        .get(self.pos + 1)
                        .map(|t| &t.kind)
                        .unwrap_or(&TokenKind::Eof)
                    {
                        self.advance(); // consume `:`
                        let variant = self.expect_ident()?;
                        // `EnumName:Variant { fields }` -- fielded enum variant construction
                        if allow_struct && self.peek() == &TokenKind::LBrace {
                            return self.parse_struct_literal(variant, start);
                        }
                        let end = self.peek_span().start;
                        return Ok(Expr::EnumAccess {
                            enum_name: name,
                            variant,
                            span: Span::new(start.start, end),
                        });
                    }
                }
                if allow_struct && self.peek() == &TokenKind::LBrace {
                    self.parse_struct_literal(name, start)
                } else {
                    Ok(Expr::Ident(name, start))
                }
            }
            TokenKind::Self_ => {
                self.advance();
                if allow_struct && self.peek() == &TokenKind::LBrace {
                    self.parse_struct_literal("Self".to_string(), start)
                } else {
                    Ok(Expr::Ident("Self".to_string(), start))
                }
            }
            TokenKind::Amp => {
                self.advance();
                let mutable = self.eat(&TokenKind::Mut);
                let e = self.parse_expr_inner(15, allow_struct)?;
                Ok(Expr::Ref {
                    mutable,
                    expr: Box::new(e),
                    span: start,
                })
            }
            TokenKind::Plus => {
                self.advance();
                let e = self.parse_expr_inner(15, allow_struct)?;
                Ok(Expr::UnOp {
                    op: UnOp::Pos,
                    operand: Box::new(e),
                    span: start,
                })
            }
            TokenKind::Minus => {
                self.advance();
                let e = self.parse_expr_inner(15, allow_struct)?;
                Ok(Expr::UnOp {
                    op: UnOp::Neg,
                    operand: Box::new(e),
                    span: start,
                })
            }
            TokenKind::Bang => {
                self.advance();
                let e = self.parse_expr_inner(15, allow_struct)?;
                Ok(Expr::UnOp {
                    op: UnOp::Not,
                    operand: Box::new(e),
                    span: start,
                })
            }
            TokenKind::LBracket => {
                self.advance();
                let mut elems = Vec::new();
                while self.peek() != &TokenKind::RBracket && self.peek() != &TokenKind::Eof {
                    elems.push(self.parse_expr(0)?);
                    self.eat(&TokenKind::Comma);
                }
                let end = self.peek_span();
                self.expect(TokenKind::RBracket)?;
                Ok(Expr::Array(elems, Span::new(start.start, end.start)))
            }
            TokenKind::Match => self.parse_match_expr(),
            TokenKind::Spawn => {
                self.advance();
                let e = self.parse_expr_inner(0, allow_struct)?;
                Ok(Expr::Spawn(Box::new(e), start))
            }
            TokenKind::Gen => self.parse_gen_expr(),
            TokenKind::LtLt => {
                let ss = self.peek_span();
                self.advance();
                let expr = self.parse_expr(0)?;
                self.expect(TokenKind::GtGt)?;
                let end = self.peek_span().start;
                Ok(Expr::GenSplice(Box::new(expr), Span::new(ss.start, end)))
            }
            found => Err(ParseError::Unexpected {
                found,
                expected: "expression".into(),
                span: start,
            }),
        }
    }

    fn parse_postfix(&mut self, lhs: Expr) -> Result<Expr, ParseError> {
        let start = lhs.span();
        match self.peek().clone() {
            TokenKind::Dot => {
                self.advance();
                let field = self.expect_field_name()?;
                if self.peek() == &TokenKind::LParen {
                    let args = self.parse_arg_list()?;
                    let span = Span::new(start.start, self.peek_span().start);
                    let callee = Expr::Field {
                        object: Box::new(lhs),
                        field,
                        span: Span::new(start.start, self.peek_span().start),
                    };
                    Ok(Expr::Call {
                        callee: Box::new(callee),
                        args,
                        span,
                    })
                } else {
                    let span = Span::new(start.start, self.peek_span().start);
                    Ok(Expr::Field {
                        object: Box::new(lhs),
                        field,
                        span,
                    })
                }
            }
            TokenKind::LParen => {
                let args = self.parse_arg_list()?;
                let span = Span::new(start.start, self.peek_span().start);
                Ok(Expr::Call {
                    callee: Box::new(lhs),
                    args,
                    span,
                })
            }
            TokenKind::LBracket => {
                self.advance();
                let idx = self.parse_expr(0)?;
                self.expect(TokenKind::RBracket)?;
                let span = Span::new(start.start, self.peek_span().start);
                Ok(Expr::Index {
                    object: Box::new(lhs),
                    index: Box::new(idx),
                    span,
                })
            }
            TokenKind::Question => {
                self.advance();
                Ok(Expr::Unwrap(Box::new(lhs), start))
            }
            TokenKind::As => {
                self.advance();
                let ty = self.parse_type()?;
                let span = Span::new(start.start, self.peek_span().start);
                Ok(Expr::As {
                    expr: Box::new(lhs),
                    ty,
                    span,
                })
            }
            _ => Ok(lhs),
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        self.expect(TokenKind::LParen)?;
        let mut args = Vec::new();
        while self.peek() != &TokenKind::RParen {
            args.push(self.parse_expr(0)?);
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        Ok(args)
    }

    fn parse_struct_literal(&mut self, ty: String, start: Span) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while self.peek() != &TokenKind::RBrace {
            let name = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let val = self.parse_expr(0)?;
            fields.push((name, val));
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;
        let span = Span::new(start.start, self.peek_span().start);
        Ok(Expr::StructLiteral { ty, fields, span })
    }

    fn parse_closure_or_paren(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span();
        self.advance();

        let is_closure = self.is_closure_params();

        if is_closure {
            let mut params = Vec::new();
            while self.peek() != &TokenKind::RParen {
                let ps = self.peek_span();
                let pname = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                let end = self.peek_span();
                params.push(Param {
                    name: pname,
                    ty,
                    mutable: false,
                    span: Span::new(ps.start, end.start),
                });
                self.eat(&TokenKind::Comma);
            }
            self.expect(TokenKind::RParen)?;
            self.expect(TokenKind::Arrow)?;
            let body = if self.peek() == &TokenKind::LBrace {
                ClosureBody::Block(self.parse_block()?)
            } else {
                ClosureBody::Expr(Box::new(self.parse_expr(0)?))
            };
            let end = self.peek_span().start;
            return Ok(Expr::Closure {
                params,
                body,
                span: Span::new(start.start, end),
            });
        }

        if self.peek() == &TokenKind::RParen {
            self.advance();

            return Ok(Expr::Tuple(vec![], start));
        }

        let first = self.parse_expr(0)?;

        if self.eat(&TokenKind::Comma) {
            let mut elems = vec![first];
            while self.peek() != &TokenKind::RParen {
                elems.push(self.parse_expr(0)?);
                self.eat(&TokenKind::Comma);
            }
            self.expect(TokenKind::RParen)?;
            let end = self.peek_span().start;
            return Ok(Expr::Tuple(elems, Span::new(start.start, end)));
        }

        self.expect(TokenKind::RParen)?;

        if self.eat(&TokenKind::Arrow) {
            return Ok(first);
        }

        Ok(first)
    }

    fn is_closure_params(&self) -> bool {
        let mut i = self.pos;
        if i < self.tokens.len() && self.tokens[i].kind == TokenKind::RParen {
            return i + 1 < self.tokens.len() && self.tokens[i + 1].kind == TokenKind::Arrow;
        }

        let mut depth = 0usize;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen => {
                    if depth == 0 {
                        return i + 1 < self.tokens.len()
                            && self.tokens[i + 1].kind == TokenKind::Arrow;
                    }
                    depth = depth.saturating_sub(1);
                }
                TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_match_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Match)?;
        let scrutinee = self.parse_expr_no_struct(0)?;
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            let pattern = self.parse_pattern()?;
            let guard = if self.eat(&TokenKind::If) {
                Some(self.parse_expr(0)?)
            } else {
                None
            };
            self.expect(TokenKind::FatArrow)?;
            let body = if self.peek() == &TokenKind::LBrace {
                let block = self.parse_block()?;
                let span = block.span;
                Expr::Block(block.stmts, span)
            } else {
                self.parse_expr(0)?
            };
            let span = pattern.span();
            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span,
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(Expr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            span: Span::new(start, end),
        })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Ident(name) if name == "_" => {
                self.advance();
                Ok(Pattern::Wildcard(start))
            }
            TokenKind::Int(n) => {
                self.advance();
                Ok(Pattern::Literal(Expr::Int(n, start)))
            }
            TokenKind::Float(f) => {
                self.advance();
                Ok(Pattern::Literal(Expr::Float(f, start)))
            }
            TokenKind::True => {
                self.advance();
                Ok(Pattern::Literal(Expr::Bool(true, start)))
            }
            TokenKind::False => {
                self.advance();
                Ok(Pattern::Literal(Expr::Bool(false, start)))
            }
            TokenKind::StringStart => {
                let e = self.parse_string_expr()?;
                Ok(Pattern::Literal(e))
            }
            TokenKind::LParen => {
                self.advance();
                let mut pats = Vec::new();
                while self.peek() != &TokenKind::RParen {
                    pats.push(self.parse_pattern()?);
                    self.eat(&TokenKind::Comma);
                }
                self.expect(TokenKind::RParen)?;
                let end = self.peek_span();
                Ok(Pattern::Tuple(pats, Span::new(start.start, end.start)))
            }
            TokenKind::Ident(name) => {
                self.advance();

                if self.peek() == &TokenKind::LBrace {
                    self.advance();
                    let mut fields = Vec::new();
                    let mut has_rest = false;
                    while self.peek() != &TokenKind::RBrace {
                        if self.eat(&TokenKind::DotDot) {
                            has_rest = true;
                            self.eat(&TokenKind::Comma);
                            break;
                        }
                        let fname = self.expect_ident()?;
                        self.expect(TokenKind::Colon)?;
                        let binding = self.expect_ident()?;
                        fields.push((fname, binding));
                        self.eat(&TokenKind::Comma);
                    }
                    self.expect(TokenKind::RBrace)?;
                    let end = self.peek_span();
                    return Ok(Pattern::Struct {
                        variant: name,
                        fields,
                        has_rest,
                        span: Span::new(start.start, end.start),
                    });
                }

                if let TokenKind::Ident(binding) = self.peek().clone() {
                    self.advance();
                    let end = self.peek_span();
                    return Ok(Pattern::TypeBinding {
                        ty: name,
                        name: binding,
                        span: Span::new(start.start, end.start),
                    });
                }
                Ok(Pattern::TypeBinding {
                    ty: "_".into(),
                    name,
                    span: start,
                })
            }
            found => Err(ParseError::Unexpected {
                found,
                expected: "pattern".into(),
                span: start,
            }),
        }
    }

    fn parse_gen_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Gen)?;
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            if self.peek() == &TokenKind::LtLt {
                let ss = self.peek_span();
                self.advance();
                let expr = self.parse_expr(0)?;
                self.expect(TokenKind::GtGt)?;
                let end = self.peek_span().start;
                stmts.push(Stmt::Expr(Expr::GenSplice(
                    Box::new(expr),
                    Span::new(ss.start, end),
                )));
            } else {
                stmts.push(self.parse_stmt()?);
            }
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(Expr::Gen {
            body: Block {
                stmts,
                span: Span::new(start, end),
            },
            span: Span::new(start, end),
        })
    }

    fn parse_string_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::StringStart)?;
        let mut segments = Vec::new();
        loop {
            match self.peek().clone() {
                TokenKind::StringEnd => {
                    self.advance();
                    break;
                }
                TokenKind::StringText(t) => {
                    self.advance();
                    segments.push(StringSegment::Text(t));
                }
                TokenKind::InterpStart => {
                    self.advance();
                    let expr = self.parse_expr(0)?;
                    self.expect(TokenKind::InterpEnd)?;
                    segments.push(StringSegment::Interp(expr));
                }
                found => {
                    return Err(ParseError::Unexpected {
                        found,
                        expected: "string content".into(),
                        span: self.peek_span(),
                    })
                }
            }
        }
        let end = self.peek_span().start;
        Ok(Expr::Str(segments, Span::new(start, end)))
    }
}

fn compound_assign_op(tok: &TokenKind) -> Option<BinOp> {
    match tok {
        TokenKind::PlusEq => Some(BinOp::Add),
        TokenKind::MinusEq => Some(BinOp::Sub),
        TokenKind::StarEq => Some(BinOp::Mul),
        TokenKind::SlashEq => Some(BinOp::Div),
        TokenKind::PercentEq => Some(BinOp::Mod),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Result<SourceFile, ParseError> {
        let tokens = Lexer::new(src)
            .tokenize()
            .map_err(|_| ParseError::LexError)?;
        Parser::new(tokens).parse_file()
    }

    #[test]
    fn empty_file_parses() {
        let file = parse("").unwrap();
        assert!(file.items.is_empty());
    }

    #[test]
    fn peek_does_not_advance() {
        let tokens = Lexer::new("def foo").tokenize().unwrap();
        let p = Parser::new(tokens);
        assert_eq!(p.peek(), &TokenKind::Def);
        assert_eq!(p.peek(), &TokenKind::Def);
    }

    #[test]
    fn parse_import() {
        let file = parse("import math { sqrt, pow }").unwrap();
        match &file.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path, vec!["math"]);
                assert_eq!(i.symbols, vec!["sqrt", "pow"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_import_wildcard() {
        let file = parse("import math { * }").unwrap();
        match &file.items[0] {
            Item::Import(i) => {
                assert_eq!(i.path, vec!["math"]);
                assert_eq!(i.symbols, vec!["*"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_export() {
        let file = parse("export { foo, Bar }").unwrap();
        match &file.items[0] {
            Item::Export(e) => assert_eq!(e.symbols, vec!["foo", "Bar"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_export_wildcard() {
        let file = parse("export { * }").unwrap();
        match &file.items[0] {
            Item::Export(e) => assert_eq!(e.symbols, vec!["*"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_annotation_use() {
        let file = parse("@static\ndef foo() -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => assert_eq!(f.annotations[0].name, "static"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_named_type() {
        let file = parse("def f(x: int) -> str {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert!(matches!(&f.params[0].ty, TypeExpr::Named { name, .. } if name == "int"));
                assert!(matches!(&f.return_type, TypeExpr::Named { name, .. } if name == "str"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_generic_type() {
        let file = parse("def f(x: Vec[int]) -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.params[0].ty {
                TypeExpr::Named { name, generics, .. } => {
                    assert_eq!(name, "Vec");
                    assert_eq!(generics.len(), 1);
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_union_type() {
        let file = parse("def f(x: int | str) -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => assert!(matches!(&f.params[0].ty, TypeExpr::Union(..))),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_simple_fn() {
        let file = parse("def add(a: int, b: int) -> int { return a + b }").unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert!(matches!(&f.return_type, TypeExpr::Named { name, .. } if name == "int"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_generic_fn() {
        let file = parse("def add[T: Addable](a: T, b: T) -> T {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert_eq!(f.generic_params.len(), 1);
                assert_eq!(f.generic_params[0].name, "T");
                let names: Vec<&str> = f.generic_params[0]
                    .bounds
                    .iter()
                    .filter_map(|b| {
                        if let TypeExpr::Named { name, .. } = b {
                            Some(name.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(names, vec!["Addable"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_variadic_fn() {
        let file = parse("def log(level: str, *messages: str) -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert!(f.variadic.is_some());
                assert_eq!(f.variadic.as_ref().unwrap().name, "messages");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_struct() {
        let src = "struct Point: Display { x: float, y: float }";
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Struct(s) => {
                assert_eq!(s.name, "Point");
                assert_eq!(s.interfaces.len(), 1);
                assert_eq!(s.fields.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_enum_plain() {
        let file = parse("enum Direction { North, South, East, West }").unwrap();
        match &file.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.name, "Direction");
                assert_eq!(e.variants.len(), 4);
                assert!(e.variants[0].fields.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_enum_with_fields() {
        let file = parse("enum Shape { Circle { radius: float }, Dot }").unwrap();
        match &file.items[0] {
            Item::Enum(e) => {
                assert_eq!(e.variants[0].name, "Circle");
                assert_eq!(e.variants[0].fields.len(), 1);
                assert_eq!(e.variants[1].name, "Dot");
                assert!(e.variants[1].fields.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_interface() {
        let src = "interface Addable { hook +(other: Self) -> Self }";
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Interface(i) => {
                assert_eq!(i.name, "Addable");
                assert_eq!(i.items.len(), 1);
                match &i.items[0].kind {
                    InterfaceItemKind::Hook {
                        name: HookName::Op(op),
                        ..
                    } => assert_eq!(op, "+"),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_impl_block() {
        let src = r#"impl Display for Point { hook to_str() -> str { return "point" } }"#;
        let file = parse(src).unwrap();
        assert!(matches!(&file.items[0], Item::ImplBlock(_)));
    }

    #[test]
    fn parse_var_decl() {
        let file = parse("def f() -> void { x: int = 5 }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::VarDecl { name, .. } => assert_eq!(name, "x"),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_mut_var_decl() {
        let file = parse("def f() -> void { mut x: int = 5 }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::VarDecl { name, mutable, .. } => {
                    assert_eq!(name, "x");
                    assert!(*mutable);
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_return_stmt() {
        let file = parse("def f() -> int { return 42 }").unwrap();
        match &file.items[0] {
            Item::Function(f) => assert!(matches!(&f.body.stmts[0], Stmt::Return { .. })),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_for_loop() {
        let file = parse("def f() -> void { for x <- items { } }").unwrap();
        match &file.items[0] {
            Item::Function(f) => assert!(matches!(&f.body.stmts[0], Stmt::For { .. })),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_try_catch() {
        let src = r#"def f() -> void {
          try { raise Err { message: "x" } }
          except Err as e { }
        }"#;
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Function(f) => assert!(matches!(&f.body.stmts[0], Stmt::TryCatch { .. })),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_integer_expr() {
        let file = parse("def f() -> int { return 42 }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Return {
                    value: Some(Expr::Int(42, _)),
                    ..
                } => {}
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_binary_add() {
        let file = parse("def f() -> int { return a + b }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Return {
                    value: Some(Expr::BinOp { op: BinOp::Add, .. }),
                    ..
                } => {}
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_precedence() {
        let file = parse("def f() -> int { return a + b * c }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Return {
                    value:
                        Some(Expr::BinOp {
                            op: BinOp::Add,
                            right,
                            ..
                        }),
                    ..
                } => {
                    assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Mul, .. }));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_method_call() {
        let file = parse("def f() -> void { x.foo(1, 2) }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Expr(Expr::Call { .. }) => {}
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_match_expr() {
        let src = r#"def f() -> str {
          return match x {
            Circle { radius: r } => "circle",
            _ => "other"
          }
        }"#;
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Return {
                    value: Some(Expr::Match { arms, .. }),
                    ..
                } => {
                    assert_eq!(arms.len(), 2);
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_closure_arrow() {
        let file = parse("def f() -> void { apply((x: int) -> x + 1) }").unwrap();
        assert!(matches!(&file.items[0], Item::Function(_)));
    }

    #[test]
    fn parse_interpolated_string_expr() {
        let file = parse(r#"def f() -> str { return "hello {name}!" }"#).unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Return {
                    value: Some(Expr::Str(segs, _)),
                    ..
                } => {
                    assert_eq!(segs.len(), 3);
                    assert!(matches!(&segs[0], StringSegment::Text(t) if t == "hello "));
                    assert!(matches!(&segs[1], StringSegment::Interp(_)));
                    assert!(matches!(&segs[2], StringSegment::Text(t) if t == "!"));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_realistic_kiln() {
        let src = r#"
import math { sqrt }

@derive(Eq, Display)
struct Point: Display {
  x: float,
  y: float

  def length() -> float {
    return sqrt(x * x + y * y)
  }
}

impl Display for Point {
  hook to_str() -> str {
    return "({x}, {y})"
  }
}

def distance(a: Point, b: Point) -> float {
  dx: float = a.x - b.x
  dy: float = a.y - b.y
  return sqrt(dx * dx + dy * dy)
}

export { Point, distance }
"#;
        let file = parse(src).unwrap();
        assert_eq!(file.items.len(), 5);
    }

    #[test]
    fn parse_tuple_destruct_assign() {
        let src = r#"def f(pair: (int, int)) -> void { (a, b) = pair }"#;
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert!(
                    matches!(&f.body.stmts[0], Stmt::Assign { .. }),
                    "expected Assign, got {:?}",
                    &f.body.stmts[0]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_lifetime_generic_param() {
        let file = parse("def longest[scope a](x: &a str, y: &a str) -> &a str {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert_eq!(f.generic_params.len(), 1);
                assert_eq!(f.generic_params[0].name, "a");
                assert!(matches!(
                    f.generic_params[0].kind,
                    GenericParamKind::Lifetime
                ));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_ref_type_with_lifetime() {
        let file = parse("def f[scope a](x: &a str) -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.params[0].ty {
                TypeExpr::Ref {
                    lifetime, mutable, ..
                } => {
                    assert_eq!(lifetime.as_deref(), Some("a"));
                    assert!(!mutable);
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_mut_ref_type_with_lifetime() {
        let file = parse("def f[scope a](x: &a mut str) -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.params[0].ty {
                TypeExpr::Ref {
                    lifetime, mutable, ..
                } => {
                    assert_eq!(lifetime.as_deref(), Some("a"));
                    assert!(mutable);
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_static_lifetime_ref() {
        let file = parse("def f() -> &static str {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.return_type {
                TypeExpr::Ref { lifetime, .. } => {
                    assert_eq!(lifetime.as_deref(), Some("static"));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_elided_ref_has_no_lifetime() {
        let file = parse("def f(x: &str) -> void {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.params[0].ty {
                TypeExpr::Ref { lifetime, .. } => assert!(lifetime.is_none()),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_lifetime_outlives_bound() {
        let file = parse("def f[scope a, scope b: a](x: &a str, y: &b str) -> &a str {}").unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert_eq!(f.generic_params.len(), 2);
                assert_eq!(f.generic_params[1].name, "b");
                let names: Vec<&str> = f.generic_params[1]
                    .bounds
                    .iter()
                    .filter_map(|b| {
                        if let TypeExpr::Named { name, .. } = b {
                            Some(name.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(names, vec!["a"]);
                assert!(matches!(
                    f.generic_params[1].kind,
                    GenericParamKind::Lifetime
                ));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_gen_block_expr() {
        let file = parse("def f() -> void { x = gen { return 5 } }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Assign {
                    value: Expr::Gen { .. },
                    ..
                } => {}
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_gen_splice_expr() {
        let file = parse("def f() -> void { x = gen { <<target>> } }").unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::Assign {
                    value: Expr::Gen { body, .. },
                    ..
                } => {
                    assert!(matches!(&body.stmts[0], Stmt::Expr(Expr::GenSplice(..))));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_compound_assign_stmt() {
        let src = "def f() -> void { x += 1 }";
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Function(f) => match &f.body.stmts[0] {
                Stmt::CompoundAssign { op: BinOp::Add, .. } => {}
                other => panic!("expected CompoundAssign(Add), got {other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_hkt_generic_params_correct_arity() {
        let src = "def traverse[G[_]: Applicative, A, B](fa: int) -> int {}";
        let file = parse(src).unwrap();
        match &file.items[0] {
            Item::Function(f) => {
                assert_eq!(f.generic_params.len(), 3, "expected 3 params: G, A, B");
                assert_eq!(f.generic_params[0].name, "G");
                assert!(matches!(
                    f.generic_params[0].kind,
                    GenericParamKind::TypeConstructor
                ));
                let names: Vec<&str> = f.generic_params[0]
                    .bounds
                    .iter()
                    .filter_map(|b| {
                        if let TypeExpr::Named { name, .. } = b {
                            Some(name.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(names, vec!["Applicative"]);
                assert_eq!(f.generic_params[1].name, "A");
                assert_eq!(f.generic_params[2].name, "B");
            }
            other => panic!("{other:?}"),
        }
    }
}
