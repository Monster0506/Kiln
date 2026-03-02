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
            TokenKind::Impl => Ok(Item::ImplBlock(self.parse_impl()?)),
            TokenKind::Annotation => Ok(Item::AnnotationDef(self.parse_annotation_def()?)),
            TokenKind::Processor => Ok(Item::ProcessorDef(self.parse_processor_def()?)),
            TokenKind::Type => Ok(Item::TypeAlias(self.parse_type_alias()?)),
            TokenKind::Const => Ok(Item::Const(self.parse_const()?)),
            TokenKind::Import => Ok(Item::Import(self.parse_import()?)),
            TokenKind::Export => Ok(Item::Export(self.parse_export()?)),
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
            symbols.push(self.expect_ident()?);
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
            symbols.push(self.expect_ident()?);
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
                let mutable = self.eat(&TokenKind::Mut);
                let inner = self.parse_type_atom()?;
                let end = inner.span();
                Ok(TypeExpr::Ref {
                    mutable,
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

                let mut generics = Vec::new();
                if self.eat(&TokenKind::LBracket) {
                    while self.peek() != &TokenKind::RBracket {
                        generics.push(self.parse_type()?);
                        self.eat(&TokenKind::Comma);
                    }
                    self.expect(TokenKind::RBracket)?;
                }
                let end = self.peek_span();
                Ok(TypeExpr::Named {
                    name,
                    generics,
                    span: Span::new(start.start, end.start),
                })
            }
            TokenKind::Void => {
                self.advance();
                Ok(TypeExpr::Named {
                    name: "void".to_string(),
                    generics: vec![],
                    span: start,
                })
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
            let name = self.expect_ident()?;
            let bound = if self.eat(&TokenKind::Colon) {
                Some(self.expect_ident()?)
            } else {
                None
            };
            let end = self.peek_span();
            params.push(GenericParam {
                name,
                bound,
                span: Span::new(start.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RBracket)?;
        Ok(params)
    }

    fn parse_fn_def(&mut self, annotations: Vec<AnnotationUse>) -> Result<FnDef, ParseError> {
        let start = self.peek_span().start;
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
            let pname = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let end = self.peek_span();
            params.push(Param {
                name: pname,
                ty,
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let return_type = self.parse_type()?;
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(FnDef {
            annotations,
            name,
            generic_params,
            params,
            variadic,
            return_type,
            body,
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
        let start = self.peek_span().start;
        self.expect(TokenKind::Struct)?;
        let name = self.expect_ident()?;
        let generic_params = self.parse_generic_params()?;
        let interfaces = self.parse_interfaces()?;
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            if self.peek() == &TokenKind::Def || self.is_annotated_def() {
                let anns = self.parse_annotation_uses()?;
                methods.push(self.parse_fn_def(anns)?);
            } else {
                fields.push(self.parse_field()?);
                self.eat(&TokenKind::Comma);
            }
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(StructDef {
            annotations,
            name,
            generic_params,
            interfaces,
            fields,
            methods,
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
            TokenKind::Minus => {
                self.advance();
                Ok(HookName::Op("-".into()))
            }
            TokenKind::Star => {
                self.advance();
                Ok(HookName::Op("*".into()))
            }
            TokenKind::Slash => {
                self.advance();
                Ok(HookName::Op("/".into()))
            }
            TokenKind::EqEq => {
                self.advance();
                Ok(HookName::Op("==".into()))
            }
            TokenKind::Spaceship => {
                self.advance();
                Ok(HookName::Op("<=>".into()))
            }
            TokenKind::LBracket => {
                self.advance();
                self.expect(TokenKind::RBracket)?;
                Ok(HookName::Op("[]".into()))
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

    fn parse_hook_sig_or_def(&mut self) -> Result<InterfaceItem, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Hook)?;
        let name = self.parse_hook_name()?;
        self.expect(TokenKind::LParen)?;
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
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let return_type = self.parse_type()?;
        let default = if self.peek() == &TokenKind::LBrace {
            Some(self.parse_block()?)
        } else {
            None
        };
        let end = self.peek_span().start;
        Ok(InterfaceItem {
            kind: InterfaceItemKind::Hook {
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
            let item = match self.peek().clone() {
                TokenKind::Hook => self.parse_hook_sig_or_def()?,
                TokenKind::Def => {
                    let method = self.parse_fn_def(vec![])?;
                    let span = method.span;
                    InterfaceItem {
                        kind: InterfaceItemKind::Method(method),
                        span,
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

    fn parse_hook_def(&mut self) -> Result<HookDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Hook)?;
        let name = self.parse_hook_name()?;
        self.expect(TokenKind::LParen)?;
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
                span: Span::new(ps.start, end.start),
            });
            self.eat(&TokenKind::Comma);
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let return_type = self.parse_type()?;
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(HookDef {
            name,
            params,
            return_type,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_impl(&mut self) -> Result<ImplBlock, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Impl)?;
        let interface = self.parse_type_atom()?;
        self.expect(TokenKind::For)?;
        let for_type = self.parse_type_atom()?;
        self.expect(TokenKind::LBrace)?;
        let mut methods = Vec::new();
        let mut hooks = Vec::new();
        while self.peek() != &TokenKind::RBrace && self.peek() != &TokenKind::Eof {
            match self.peek().clone() {
                TokenKind::Hook => hooks.push(self.parse_hook_def()?),
                TokenKind::Def => methods.push(self.parse_fn_def(vec![])?),
                found => {
                    return Err(ParseError::Unexpected {
                        found,
                        expected: "hook or def in impl block".into(),
                        span: self.peek_span(),
                    })
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        let end = self.peek_span().start;
        Ok(ImplBlock {
            interface,
            for_type,
            methods,
            hooks,
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
            span: Span::new(ps.start, pe.start),
        };
        self.expect(TokenKind::RParen)?;
        let body = self.parse_block()?;
        let end = self.peek_span().start;
        Ok(ProcessorDef {
            annotation_name,
            target_param,
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

    fn parse_const(&mut self) -> Result<ConstDef, ParseError> {
        let start = self.peek_span().start;
        self.expect(TokenKind::Const)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        self.expect(TokenKind::Eq)?;
        let value = self.parse_expr(0)?;
        let end = self.peek_span().start;
        Ok(ConstDef {
            name,
            ty,
            value,
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
                let value = self.parse_expr(0)?;
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
            TokenKind::Ident(_) => self.parse_ident_led_stmt(),
            _ => Ok(Stmt::Expr(self.parse_expr(0)?)),
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

        let expr = self.parse_infix(lhs, 0)?;
        Ok(Stmt::Expr(expr))
    }

    fn infix_bp(op: &TokenKind) -> Option<(u8, u8)> {
        match op {
            TokenKind::PipePipe => Some((1, 2)),
            TokenKind::AmpAmp => Some((3, 4)),
            TokenKind::EqEq => Some((5, 6)),
            TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::LtEq
            | TokenKind::GtEq
            | TokenKind::Spaceship => Some((7, 8)),
            TokenKind::Plus | TokenKind::Minus => Some((9, 10)),
            TokenKind::Star | TokenKind::Slash => Some((11, 12)),
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
        self.parse_infix(lhs, min_bp)
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

    fn parse_infix(&mut self, mut lhs: Expr, min_bp: u8) -> Result<Expr, ParseError> {
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
                let rhs = self.parse_expr(rbp)?;
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
            TokenKind::EqEq => BinOp::Eq,
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
                if allow_struct && self.peek() == &TokenKind::LBrace {
                    self.parse_struct_literal(name, start)
                } else {
                    Ok(Expr::Ident(name, start))
                }
            }
            TokenKind::Minus => {
                self.advance();
                let e = self.parse_expr(15)?;
                Ok(Expr::UnOp {
                    op: UnOp::Neg,
                    operand: Box::new(e),
                    span: start,
                })
            }
            TokenKind::Bang => {
                self.advance();
                let e = self.parse_expr(15)?;
                Ok(Expr::UnOp {
                    op: UnOp::Not,
                    operand: Box::new(e),
                    span: start,
                })
            }
            TokenKind::Match => self.parse_match_expr(),
            TokenKind::Spawn => {
                self.advance();
                let e = self.parse_expr(0)?;
                Ok(Expr::Spawn(Box::new(e), start))
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
                let field = self.expect_ident()?;
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
            let body = self.parse_expr(0)?;
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
                    while self.peek() != &TokenKind::RBrace {
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
    fn parse_export() {
        let file = parse("export { foo, Bar }").unwrap();
        match &file.items[0] {
            Item::Export(e) => assert_eq!(e.symbols, vec!["foo", "Bar"]),
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
                assert_eq!(f.generic_params[0].bound.as_deref(), Some("Addable"));
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
}
