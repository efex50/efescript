//! Parser of `.efa` (grammar: `docs/efa-syntax.md`, section 2). Turns the lexer's tokens into
//! the [`ast`]. Purely syntactic: names are not resolved, mnemonics are not checked, operand
//! arity and types are the lowering's business.
//!
//! Errors are recorded and parsing resumes at the next line. Inside a function body a bad
//! statement is skipped up to its end of line (or up to a `}` that stands alone), a function
//! whose `}` is missing is reported once (at the next top-level keyword or at the end of the
//! file). The parser never panics: tokens are only read through checked accessors, and every
//! loop is guaranteed to make progress.

use super::ast::{
    AParam, AType, CallSugar, ConstDecl, ConstExpr, DataDecl, DataKind, FieldDecl, FuncDecl, GlobalDecl, Ident,
    EventForm, EventImportDecl, ImportDecl, Inst, Item, MemoryDecl, Module, Operand, Param, ProvideDecl,
    ProvideEventDecl, ScratchDecl, Signature,
    Span, Stmt, StmtKind, StructDecl, TableDecl, TableEntry, Ty, TypeRef, UseDecl,
};
use super::diag::Diagnostic;
use super::lexer::{lex, Token, TokenKind};
use efevm::{Op, Shape};

/// Maximum nesting of operands (`f(g(h(...)))`), so hostile input cannot overflow the stack.
const MAX_DEPTH: u32 = 64;

/// Longest constant expression, in tokens: the evaluator recurses over the tree.
const MAX_EXPR_TOKENS: usize = 1000;

/// Words that start a top-level item; none of them is a valid first word of a statement.
const ITEM_KEYWORDS: [&str; 12] = [
    "import", "memory", "const", "global", "table", "fn", "export", "use", "pub", "provide", "struct", "scratch",
];

/// Item keywords added for multi-file programs (efa-syntax 16.9): reserved, so a data item
/// named after one of them gets an error that says so.
const NEW_KEYWORDS: [&str; 5] = ["use", "pub", "provide", "struct", "scratch"];

/// The operand position (0-based) of an instruction that is a constant expression: the offset
/// of a load or store, the value of a `const.T`.
fn const_operand_index(mnemonic: &str) -> Option<usize> {
    let op = Op::ALL.iter().find(|o| o.name() == mnemonic)?;
    match op.shape() {
        Shape::Load | Shape::Store => Some(2),
        Shape::ConstI32 | Shape::ConstI64 | Shape::ConstF32 | Shape::ConstF64 => Some(1),
        _ => None,
    }
}

/// Can `kind` follow an operand inside one constant expression? (An operator, or a literal
/// that starts with `-`, which the lexer reads as a negative number.)
fn continues_expr(kind: &TokenKind) -> bool {
    match kind {
        TokenKind::Plus
        | TokenKind::Minus
        | TokenKind::Star
        | TokenKind::Slash
        | TokenKind::Percent
        | TokenKind::Amp
        | TokenKind::Pipe
        | TokenKind::Caret
        | TokenKind::Shl
        | TokenKind::Shr => true,
        TokenKind::Int(v) => *v < 0,
        TokenKind::Float(t) => t.starts_with('-') && !t.starts_with("-nan:"),
        _ => false,
    }
}

/// A syntax error that has already been recorded; the caller recovers.
struct Abort;

type PResult<T> = Result<T, Abort>;

static EOF_TOKEN: Token = Token { kind: TokenKind::Eof, span: Span::none() };

pub(super) fn parse(tokens: &[Token], src: &str, file: &str) -> (Module, Vec<Diagnostic>) {
    let mut p = Parser {
        toks: tokens,
        pos: 0,
        prev: Span::none(),
        file,
        lines: src.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect(),
        diags: Vec::new(),
        depth: 0,
        in_body: false,
        // Lines where the lexer already reported an error: a second error there is noise.
        reported: lex(src, file).1.iter().map(|d| d.line).collect(),
    };
    let module = p.module();
    (module, p.diags)
}

/// How a token is named in "found ..." messages.
fn describe(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Name(n) => format!("`{}`", n),
        TokenKind::Int(v) => format!("`{}`", v),
        TokenKind::Float(s) => format!("`{}`", s),
        TokenKind::Char(_) => "a character literal".to_string(),
        TokenKind::Str(_) => "a string literal".to_string(),
        TokenKind::LBrace => "`{`".to_string(),
        TokenKind::RBrace => "`}`".to_string(),
        TokenKind::LParen => "`(`".to_string(),
        TokenKind::RParen => "`)`".to_string(),
        TokenKind::LBracket => "`[`".to_string(),
        TokenKind::RBracket => "`]`".to_string(),
        TokenKind::Comma => "`,`".to_string(),
        TokenKind::Colon => "`:`".to_string(),
        TokenKind::Equals => "`=`".to_string(),
        TokenKind::Arrow => "`->`".to_string(),
        TokenKind::Amp => "`&`".to_string(),
        TokenKind::Plus => "`+`".to_string(),
        TokenKind::Minus => "`-`".to_string(),
        TokenKind::Star => "`*`".to_string(),
        TokenKind::Slash => "`/`".to_string(),
        TokenKind::Percent => "`%`".to_string(),
        TokenKind::Pipe => "`|`".to_string(),
        TokenKind::Caret => "`^`".to_string(),
        TokenKind::Tilde => "`~`".to_string(),
        TokenKind::Shl => "`<<`".to_string(),
        TokenKind::Shr => "`>>`".to_string(),
        TokenKind::Newline => "end of line".to_string(),
        TokenKind::Eof => "end of file".to_string(),
    }
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    /// Span of the last consumed token that is not a Newline or Eof.
    prev: Span,
    file: &'a str,
    lines: Vec<&'a str>,
    diags: Vec<Diagnostic>,
    depth: u32,
    /// Inside a function body: a `}` on a statement line ends the statement (and the function).
    in_body: bool,
    /// Lines that already carry a diagnostic; at most one is reported per line.
    reported: Vec<u32>,
}

impl<'a> Parser<'a> {
    // ---- token access -------------------------------------------------------------------

    fn tok_at(&self, n: usize) -> &'a Token {
        let toks: &'a [Token] = self.toks;
        toks.get(self.pos.saturating_add(n)).unwrap_or(&EOF_TOKEN)
    }

    fn tok(&self) -> &'a Token {
        self.tok_at(0)
    }

    fn kind(&self) -> &'a TokenKind {
        &self.tok().kind
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.kind() == kind
    }

    fn at_eol(&self) -> bool {
        matches!(self.kind(), TokenKind::Newline | TokenKind::Eof)
    }

    /// Consumes the current token (never past the end) and returns its span.
    fn bump(&mut self) -> Span {
        let tok = self.tok();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        if !matches!(tok.kind, TokenKind::Newline | TokenKind::Eof) {
            self.prev = tok.span;
        }
        tok.span
    }

    /// Consumes a Newline if there is one.
    fn eat_newline(&mut self) {
        if matches!(self.kind(), TokenKind::Newline) {
            self.bump();
        }
    }

    /// Is token `i` a `}` that stands alone on its line (as far as the token stream shows)?
    fn is_closing_brace(&self, i: usize) -> bool {
        let kind_at = |j: usize| self.toks.get(j).map(|t| &t.kind);
        matches!(kind_at(i), Some(TokenKind::RBrace))
            && matches!(kind_at(i + 1), None | Some(TokenKind::Newline) | Some(TokenKind::Eof))
            && (i == 0
                || matches!(kind_at(i - 1), Some(TokenKind::Newline) | Some(TokenKind::Comma)))
    }

    // ---- diagnostics --------------------------------------------------------------------

    fn push_diag(&mut self, span: Span, msg: String, note: Option<String>) {
        let mut span = span;
        if span.line == 0 {
            if let Some(last) = self.toks.last() {
                span = last.span;
            }
        }
        if span.line != 0 && self.reported.contains(&span.line) {
            return;
        }
        self.reported.push(span.line);
        let text = if span.line == 0 {
            ""
        } else {
            self.lines.get(span.line as usize - 1).copied().unwrap_or("")
        };
        let mut d = Diagnostic::with_line(self.file, text, span, msg);
        d.note = note;
        self.diags.push(d);
    }

    /// Records an error and returns the marker that aborts the current construct.
    fn error(&mut self, span: Span, msg: impl Into<String>) -> Abort {
        self.push_diag(span, msg.into(), None);
        Abort
    }

    /// Records an error at the current token: "expected WHAT, found TOKEN".
    fn expected(&mut self, what: &str) -> Abort {
        let tok = self.tok();
        let msg = format!("expected {}, found {}", what, describe(&tok.kind));
        self.error(tok.span, msg)
    }

    fn expect_tok(&mut self, kind: TokenKind, what: &str) -> PResult<Span> {
        if self.at(&kind) {
            Ok(self.bump())
        } else {
            Err(self.expected(what))
        }
    }

    fn ident(&mut self, what: &str) -> PResult<Ident> {
        if let TokenKind::Name(n) = self.kind() {
            let span = self.bump();
            Ok(Ident { name: n.clone(), span })
        } else {
            Err(self.expected(what))
        }
    }

    /// The statement must end here: consumes the Newline, or reports what was found instead.
    fn finish_line(&mut self, after: &str) -> PResult<()> {
        if self.at_eol() || self.brace_ends_stmt() {
            self.eat_newline();
            Ok(())
        } else {
            Err(self.expected(&format!("end of line after {}", after)))
        }
    }

    /// In a body, a `}` after a statement closes the function (after an error); it is left
    /// for the body loop.
    fn brace_ends_stmt(&mut self) -> bool {
        if self.in_body && matches!(self.kind(), TokenKind::RBrace) {
            let span = self.tok().span;
            self.push_diag(span, "`}` must stand alone on its line".to_string(), None);
            true
        } else {
            false
        }
    }

    // ---- recovery -----------------------------------------------------------------------

    fn skip_line(&mut self) {
        while !self.at_eol() {
            self.bump();
        }
        self.eat_newline();
    }

    /// Skips through the next `}` that stands alone on its line (or to the end of the file).
    fn skip_body(&mut self) {
        loop {
            if matches!(self.kind(), TokenKind::Eof) {
                return;
            }
            if self.is_closing_brace(self.pos) {
                self.bump();
                self.eat_newline();
                return;
            }
            self.bump();
        }
    }

    /// After a failed top-level item: skip the line; when it opens a block (ends with `{`),
    /// skip the whole block too so its lines do not produce a flood of errors.
    fn recover_top(&mut self) {
        while !self.at_eol() {
            self.bump();
        }
        let opens_block =
            self.pos > 0 && matches!(self.toks.get(self.pos - 1).map(|t| &t.kind), Some(TokenKind::LBrace));
        self.eat_newline();
        if opens_block {
            self.skip_body();
        }
    }

    /// After a failed statement: skip to the end of the line, but never past a `}` that
    /// stands alone (a comma continuation may have swallowed the newline before it).
    fn recover_stmt(&mut self) {
        while !self.at_eol() && !self.is_closing_brace(self.pos) {
            self.bump();
        }
        self.eat_newline();
    }

    // ---- module -------------------------------------------------------------------------

    fn module(&mut self) -> Module {
        let mut items = Vec::new();
        loop {
            match self.kind() {
                TokenKind::Newline => {
                    self.bump();
                    continue;
                }
                TokenKind::Eof => break,
                _ => {}
            }
            if self.pos >= self.toks.len() {
                break;
            }
            let start = self.pos;
            match self.item() {
                Ok(item) => items.push(item),
                Err(Abort) => self.recover_top(),
            }
            if self.pos == start {
                self.bump();
            }
        }
        Module { items }
    }

    fn item(&mut self) -> PResult<Item> {
        let tok = self.tok();
        let name = match &tok.kind {
            TokenKind::Name(n) => n.as_str(),
            TokenKind::RBrace => {
                return Err(self.error(tok.span, "unexpected `}`: no function is open here"));
            }
            _ => {
                return Err(self.expected(
                    "an item (`import`, `memory`, `const`, `global`, `table`, `fn`, `export` or a data definition)",
                ));
            }
        };
        if NEW_KEYWORDS.contains(&name)
            && matches!(&self.tok_at(1).kind, TokenKind::Name(k) if DataKind::from_name(k).is_some())
        {
            return Err(self.error(tok.span, format!("`{name}` is a reserved word and cannot be a data name")));
        }
        match name {
            "import" => self.import(),
            "memory" => self.memory(),
            "const" => self.constant(false, tok.span),
            "global" => self.global(false, false, tok.span),
            "table" => self.table(),
            "fn" => self.func(false, false, tok.span),
            "use" => self.use_item(),
            "provide" => self.provide(),
            "struct" => self.struct_item(false, false, tok.span),
            "scratch" => self.scratch(),
            "export" => {
                self.bump();
                match self.kind() {
                    TokenKind::Name(n) if n == "fn" => self.func(true, false, tok.span),
                    TokenKind::Name(n) if n == "global" => self.global(true, false, tok.span),
                    TokenKind::Name(n) if n == "struct" => self.struct_item(false, true, tok.span),
                    // `export struct` is accepted too; the message keeps its E2 wording.
                    _ => Err(self.expected("`fn` or `global` after `export`")),
                }
            }
            "pub" => {
                self.bump();
                let next = self.tok();
                match &next.kind {
                    TokenKind::Name(n) if n == "fn" => self.func(false, true, tok.span),
                    TokenKind::Name(n) if n == "global" => self.global(false, true, tok.span),
                    TokenKind::Name(n) if n == "const" => self.constant(true, tok.span),
                    TokenKind::Name(n) if n == "struct" => self.struct_item(true, false, tok.span),
                    TokenKind::Name(n)
                        if ["import", "use", "memory", "table", "scratch", "provide", "export", "pub"]
                            .contains(&n.as_str()) =>
                    {
                        let what = n.clone();
                        Err(self.error(
                            next.span,
                            format!("`pub` is not allowed on `{what}`"),
                        ))
                    }
                    TokenKind::Name(_) if matches!(&self.tok_at(1).kind, TokenKind::Name(k) if DataKind::from_name(k).is_some()) => {
                        self.data(true, tok.span)
                    }
                    _ => Err(self.expected("`fn`, `const`, `global`, `struct` or a data definition after `pub`")),
                }
            }
            _ => self.data(false, Span::none()),
        }
    }

    /// `( [&S] )` of an event: the payload struct, or `None` for `()`.
    fn event_payload(&mut self) -> PResult<Option<(Ident, Span)>> {
        self.expect_tok(TokenKind::LParen, "`(` and the payload of the event: `(&Struct)` or `()`")?;
        if matches!(self.kind(), TokenKind::RParen) {
            self.bump();
            return Ok(None);
        }
        if !matches!(self.kind(), TokenKind::Amp) {
            return Err(self.expected("`&Struct` (the payload) or `)` for an event without payload"));
        }
        let amp = self.bump();
        let s = self.ident("a struct name after `&`")?;
        let span = amp.join(s.span);
        self.expect_tok(TokenKind::RParen, "`)` after the payload struct")?;
        Ok(Some((s, span)))
    }

    /// `import event NS.NAME [( [&S] )]`, after `import` was consumed and `event` is current.
    fn event_import(&mut self, start: Span) -> PResult<Item> {
        self.bump(); // `event`
        let qualified = self.ident("a qualified event name like `game.score_changed`")?;
        if !qualified.name.contains('.') {
            self.push_diag(
                qualified.span,
                format!("event name `{}` must be qualified as `namespace.name`", qualified.name),
                None,
            );
        }
        let form = if matches!(self.kind(), TokenKind::LParen) {
            match self.event_payload()? {
                Some((s, span)) => EventForm::Struct(s, span),
                None => EventForm::NoPayload,
            }
        } else {
            EventForm::FromIndex
        };
        let span = start.join(self.prev);
        self.finish_line("the event import")?;
        Ok(Item::EventImport(EventImportDecl { span, qualified, form }))
    }

    /// `provide event NAME ( [&S] ), STRING`, with `event` current.
    fn provide_event(&mut self, start: Span) -> PResult<Item> {
        self.bump(); // `event`
        let name = self.ident("an event name after `provide event`")?;
        let payload = self.event_payload()?;
        self.expect_tok(TokenKind::Comma, "`,` and the doc string of the event")?;
        let (doc, doc_span) = match self.kind() {
            TokenKind::Str(b) => {
                let b = b.clone();
                (b, self.bump())
            }
            _ => return Err(self.expected("the doc string of the event")),
        };
        let span = start.join(self.prev);
        self.finish_line("the doc string")?;
        Ok(Item::ProvideEvent(ProvideEventDecl { span, name, payload, doc, doc_span }))
    }

    fn import(&mut self) -> PResult<Item> {
        let start = self.bump();
        if matches!(self.kind(), TokenKind::Name(n) if n == "event")
            && matches!(self.tok_at(1).kind, TokenKind::Name(_))
        {
            return self.event_import(start);
        }
        let qualified = self.ident("a qualified name like `std.println` after `import`")?;
        if !qualified.name.contains('.') {
            self.push_diag(
                qualified.span,
                format!("import name `{}` must be qualified as `namespace.name`", qualified.name),
                None,
            );
        }
        let has_sig = matches!(self.kind(), TokenKind::LParen);
        let (params, results) = if has_sig {
            let params = self.type_list_paren("`(` after the import name")?;
            (params, self.opt_results()?)
        } else {
            // `import ns.name`: the signature comes from the API index.
            (Vec::new(), Vec::new())
        };
        let span = start.join(self.prev);
        self.finish_line(if has_sig { "the import signature" } else { "the import name" })?;
        Ok(Item::Import(ImportDecl { span, qualified, params, results, has_sig }))
    }

    fn memory(&mut self) -> PResult<Item> {
        let start = self.bump();
        let min = self.const_operand()?;
        let max = if self.at_eol() { None } else { Some(self.const_operand()?) };
        let span = start.join(self.prev);
        self.finish_line("the memory operands")?;
        Ok(Item::Memory(MemoryDecl { span, min, max }))
    }

    fn constant(&mut self, public: bool, start: Span) -> PResult<Item> {
        self.bump(); // `const`
        let name = self.ident("a constant name after `const`")?;
        self.expect_tok(TokenKind::Equals, "`=` after the constant name")?;
        let value = self.const_operand()?;
        let span = start.join(self.prev);
        self.finish_line("the constant value")?;
        Ok(Item::Const(ConstDecl { span, public, name, value }))
    }

    fn global(&mut self, exported: bool, public: bool, start: Span) -> PResult<Item> {
        self.bump(); // `global`
        let mut mutable = false;
        if matches!(self.kind(), TokenKind::Name(n) if n == "mut")
            && !matches!(self.tok_at(1).kind, TokenKind::Colon)
        {
            self.bump();
            mutable = true;
        }
        let name = self.ident("a global name")?;
        self.expect_tok(TokenKind::Colon, "`:` after the global name")?;
        let ty = self.parse_type()?;
        self.expect_tok(TokenKind::Equals, "`=` after the global type")?;
        let init = self.const_operand()?;
        let span = start.join(self.prev);
        self.finish_line("the global initialiser")?;
        Ok(Item::Global(GlobalDecl { span, exported, public, mutable, name, ty, init }))
    }

    fn table(&mut self) -> PResult<Item> {
        let start = self.bump();
        self.expect_tok(TokenKind::LBracket, "`[` after `table`")?;
        let mut entries = Vec::new();
        if matches!(self.kind(), TokenKind::RBracket) {
            self.bump();
        } else {
            loop {
                match self.kind() {
                    TokenKind::Name(n) if n == "null" => {
                        let span = self.bump();
                        entries.push(TableEntry::Null(span));
                    }
                    TokenKind::Name(_) => {
                        let id = self.ident("a function name or `null`")?;
                        entries.push(TableEntry::Func(id));
                    }
                    _ => return Err(self.expected("a function name or `null`")),
                }
                match self.kind() {
                    TokenKind::Comma => {
                        self.bump();
                    }
                    TokenKind::RBracket => {
                        self.bump();
                        break;
                    }
                    _ => return Err(self.expected("`,` or `]` after the table entry")),
                }
            }
        }
        let span = start.join(self.prev);
        self.finish_line("the table")?;
        Ok(Item::Table(TableDecl { span, entries }))
    }

    /// A data item; `start` is the span of a leading `pub` (none without it).
    fn data(&mut self, public: bool, start: Span) -> PResult<Item> {
        let name = self.ident("a data name")?;
        let (kind, kind_span) = match self.kind() {
            TokenKind::Name(n) => match DataKind::from_name(n) {
                Some(k) => {
                    let s = self.bump();
                    (k, s)
                }
                None => return Err(self.data_kind_expected(&name.name)),
            },
            _ => return Err(self.data_kind_expected(&name.name)),
        };
        let values = self.operands_to_eol(|_| true)?;
        let first = if public { start } else { name.span };
        let span = first.join(self.prev);
        self.eat_newline();
        Ok(Item::Data(DataDecl { span, public, name, kind, kind_span, values }))
    }

    fn data_kind_expected(&mut self, name: &str) -> Abort {
        self.expected(&format!("`db`, `dw`, `dd`, `dq` or `resb` after the data name `{}`", name))
    }

    // ---- functions ----------------------------------------------------------------------

    fn func(&mut self, exported: bool, public: bool, start: Span) -> PResult<Item> {
        self.bump(); // `fn`
        let name = self.ident("a function name after `fn`")?;
        self.expect_tok(TokenKind::LParen, "`(` after the function name")?;
        let mut params = Vec::new();
        if matches!(self.kind(), TokenKind::RParen) {
            self.bump();
        } else {
            loop {
                let pname = self.ident("a parameter name")?;
                self.expect_tok(TokenKind::Colon, "`:` after the parameter name")?;
                let ty = self.parse_type()?;
                params.push(Param { name: pname, ty });
                match self.kind() {
                    TokenKind::Comma => {
                        self.bump();
                    }
                    TokenKind::RParen => {
                        self.bump();
                        break;
                    }
                    _ => return Err(self.expected("`,` or `)` after the parameter")),
                }
            }
        }
        let results = self.opt_results()?;
        // `{` ends the header line. A `{` alone on the next line is reported but accepted.
        if matches!(self.kind(), TokenKind::Newline) && matches!(self.tok_at(1).kind, TokenKind::LBrace) {
            self.bump();
            let brace = self.tok().span;
            self.push_diag(brace, "`{` must be on the same line as the function header".to_string(), None);
        }
        let brace = self.expect_tok(TokenKind::LBrace, "`{` to open the function body")?;
        let span = start.join(brace);
        if !self.at_eol() {
            let _ = self.expected("end of line after `{`");
        } else {
            self.eat_newline();
        }
        let (body, end) = self.body(&name);
        Ok(Item::Func(FuncDecl { span, exported, public, name, params, results, body, end }))
    }

    // ---- multi-file items (efa-syntax 16) -------------------------------------------------

    /// `use STRING [as NAME]`.
    fn use_item(&mut self) -> PResult<Item> {
        let start = self.bump(); // `use`
        let (path, path_span) = match self.kind() {
            TokenKind::Str(b) => {
                let b = b.clone();
                (b, self.bump())
            }
            _ => return Err(self.expected("a path string after `use`, like `use \"util/math.efa\"`")),
        };
        let alias = if matches!(self.kind(), TokenKind::Name(n) if n == "as") {
            self.bump();
            Some(self.ident("an alias name after `as`")?)
        } else {
            None
        };
        let span = start.join(self.prev);
        self.finish_line(if alias.is_some() { "the alias" } else { "the path" })?;
        Ok(Item::Use(UseDecl { span, path, path_span, alias }))
    }

    /// `scratch N`.
    fn scratch(&mut self) -> PResult<Item> {
        let start = self.bump(); // `scratch`
        let size = self.const_operand()?;
        let span = start.join(self.prev);
        self.finish_line("the scratch size")?;
        Ok(Item::Scratch(ScratchDecl { span, size }))
    }

    /// `NAME: atype` with `atype` a type word or `&NAME`.
    fn aparam(&mut self) -> PResult<AParam> {
        let name = self.ident("a parameter name")?;
        self.expect_tok(TokenKind::Colon, "`:` after the parameter name")?;
        let ty = if matches!(self.kind(), TokenKind::Amp) {
            let amp = self.bump();
            let s = self.ident("a struct name after `&`")?;
            let span = amp.join(s.span);
            AType::Ref(s, span)
        } else {
            AType::Word(self.ident("a type (`i32 i64 f32 f64 bool entity str bytes` or `&Struct`)")?)
        };
        Ok(AParam { name, ty })
    }

    /// After a consumed `(`: `aparam { , aparam }` and the `)` (an empty list is allowed).
    fn aparams(&mut self) -> PResult<Vec<AParam>> {
        let mut v = Vec::new();
        if matches!(self.kind(), TokenKind::RParen) {
            self.bump();
            return Ok(v);
        }
        loop {
            v.push(self.aparam()?);
            match self.kind() {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::RParen => {
                    self.bump();
                    return Ok(v);
                }
                _ => return Err(self.expected("`,` or `)` after the parameter")),
            }
        }
    }

    /// `provide NAME(params) [-> (results)] [= FUNC], STRING`.
    fn provide(&mut self) -> PResult<Item> {
        let start = self.bump(); // `provide`
        if matches!(self.kind(), TokenKind::Name(n) if n == "event")
            && matches!(self.tok_at(1).kind, TokenKind::Name(_))
        {
            return self.provide_event(start);
        }
        let name = self.ident("a provide name after `provide`")?;
        self.expect_tok(TokenKind::LParen, "`(` after the provide name")?;
        let params = self.aparams()?;
        let mut results = Vec::new();
        if matches!(self.kind(), TokenKind::Arrow) {
            self.bump();
            self.expect_tok(TokenKind::LParen, "`(` after `->`: provide results are named, as in `-> (n: i64)`")?;
            results = self.aparams()?;
        }
        let target = if matches!(self.kind(), TokenKind::Equals) {
            self.bump();
            Some(self.ident("a function name after `=`")?)
        } else {
            None
        };
        self.expect_tok(TokenKind::Comma, "`,` and the doc string of the provide")?;
        let (doc, doc_span) = match self.kind() {
            TokenKind::Str(b) => {
                let b = b.clone();
                (b, self.bump())
            }
            _ => return Err(self.expected("the doc string of the provide")),
        };
        let span = start.join(self.prev);
        self.finish_line("the doc string")?;
        Ok(Item::Provide(ProvideDecl { span, name, params, results, target, doc, doc_span }))
    }

    /// `[pub | export] struct NAME [, STRING] {` fields `}`.
    fn struct_item(&mut self, public: bool, exported: bool, start: Span) -> PResult<Item> {
        self.bump(); // `struct`
        let name = self.ident("a struct name after `struct`")?;
        let doc = if matches!(self.kind(), TokenKind::Comma) {
            self.bump();
            match self.kind() {
                TokenKind::Str(b) => {
                    let b = b.clone();
                    self.bump();
                    Some(b)
                }
                _ => return Err(self.expected("the doc string of the struct")),
            }
        } else {
            None
        };
        let brace = self.expect_tok(TokenKind::LBrace, "`{` to open the struct")?;
        let span = start.join(brace);
        if !self.at_eol() {
            return Err(self.expected("end of line after `{`"));
        }
        self.eat_newline();
        let mut fields = Vec::new();
        loop {
            let tok = self.tok();
            match &tok.kind {
                TokenKind::Newline => {
                    self.bump();
                    continue;
                }
                TokenKind::Eof => {
                    let msg = format!("expected `}}` to close struct `{}`, found end of file", name.name);
                    self.push_diag(tok.span, msg, Some(format!("the struct starts at line {}", name.span.line)));
                    return Ok(Item::Struct(StructDecl { span, public, exported, name, doc, fields, end: tok.span }));
                }
                TokenKind::RBrace => {
                    let end = self.bump();
                    if self.at_eol() {
                        self.eat_newline();
                    } else {
                        let _ = self.expected("end of line after `}`");
                        self.skip_line();
                    }
                    return Ok(Item::Struct(StructDecl { span, public, exported, name, doc, fields, end }));
                }
                _ => {}
            }
            let at = self.pos;
            match self.field() {
                Ok(f) => fields.push(f),
                Err(Abort) => self.recover_stmt(),
            }
            if self.pos == at {
                self.bump();
            }
        }
    }

    /// `NAME: PRIM [ "[" operand "]" ] [, STRING]`.
    fn field(&mut self) -> PResult<FieldDecl> {
        let name = self.ident("a field name")?;
        self.expect_tok(TokenKind::Colon, "`:` after the field name")?;
        let prim = self.ident("a field type (`u8 i8 u16 i16 u32 i32 u64 i64 f32 f64 bool entity`)")?;
        let count = if matches!(self.kind(), TokenKind::LBracket) {
            self.bump();
            let n = self.const_operand()?;
            self.expect_tok(TokenKind::RBracket, "`]` after the array length")?;
            Some(n)
        } else {
            None
        };
        let doc = if matches!(self.kind(), TokenKind::Comma) {
            self.bump();
            match self.kind() {
                TokenKind::Str(b) => {
                    let b = b.clone();
                    self.bump();
                    Some(b)
                }
                _ => return Err(self.expected("the doc string of the field")),
            }
        } else {
            None
        };
        let span = name.span.join(self.prev);
        if self.at_eol() {
            self.eat_newline();
        } else if !self.is_closing_brace(self.pos) {
            return Err(self.expected("end of line after the field"));
        }
        Ok(FieldDecl { span, name, prim, count, doc })
    }

    /// Parses statements up to the closing `}`; returns them with the span of the brace.
    fn body(&mut self, func: &Ident) -> (Vec<Stmt>, Span) {
        let saved = self.in_body;
        self.in_body = true;
        let r = self.body_inner(func);
        self.in_body = saved;
        r
    }

    fn body_inner(&mut self, func: &Ident) -> (Vec<Stmt>, Span) {
        let mut body = Vec::new();
        loop {
            let tok = self.tok();
            match &tok.kind {
                TokenKind::Newline => {
                    self.bump();
                    continue;
                }
                TokenKind::Eof => {
                    self.missing_brace(func, "end of file", tok.span);
                    return (body, tok.span);
                }
                TokenKind::RBrace => {
                    let end = self.bump();
                    if self.at_eol() {
                        self.eat_newline();
                    } else {
                        let _ = self.expected("end of line after `}`");
                        self.skip_line();
                    }
                    return (body, end);
                }
                TokenKind::Name(n)
                    if ITEM_KEYWORDS.contains(&n.as_str())
                        && !matches!(self.tok_at(1).kind, TokenKind::Colon) =>
                {
                    self.missing_brace(func, &format!("`{}`", n), tok.span);
                    return (body, tok.span);
                }
                _ => {}
            }
            let start = self.pos;
            if self.stmt(&mut body).is_err() {
                self.recover_stmt();
            }
            if self.pos == start {
                self.bump();
            }
        }
    }

    fn missing_brace(&mut self, func: &Ident, found: &str, span: Span) {
        let msg = format!("expected `}}` to close function `{}`, found {}", func.name, found);
        let note = format!("the function starts at line {}", func.span.line);
        self.push_diag(span, msg, Some(note));
    }

    /// One statement line: labels, then optionally `reg`, `nregs` or an instruction.
    fn stmt(&mut self, body: &mut Vec<Stmt>) -> PResult<()> {
        loop {
            let tok = self.tok();
            if let TokenKind::Name(n) = &tok.kind {
                if matches!(self.tok_at(1).kind, TokenKind::Colon) {
                    self.bump();
                    let colon = self.bump();
                    let id = Ident { name: n.clone(), span: tok.span };
                    body.push(Stmt { span: tok.span.join(colon), kind: StmtKind::Label(id) });
                    continue;
                }
            }
            break;
        }
        if self.at_eol() {
            self.eat_newline();
            return Ok(());
        }
        let tok = self.tok();
        let word = match &tok.kind {
            TokenKind::Name(n) => n.as_str(),
            TokenKind::RBrace => {
                // The brace still closes the function (the body loop sees it next).
                self.push_diag(tok.span, "`}` must stand alone on its line".to_string(), None);
                return Ok(());
            }
            _ => return Err(self.expected("an instruction")),
        };
        match word {
            "reg" => {
                self.bump();
                let mut names = Vec::new();
                loop {
                    names.push(self.ident("a register name")?);
                    if matches!(self.kind(), TokenKind::Comma) {
                        self.bump();
                        continue;
                    }
                    break;
                }
                let span = tok.span.join(self.prev);
                self.finish_line("the register name (separate names with `,`)")?;
                body.push(Stmt { span, kind: StmtKind::Reg(names) });
            }
            "nregs" => {
                self.bump();
                let value = self.parse_operand()?;
                let span = tok.span.join(self.prev);
                self.finish_line("the `nregs` value")?;
                body.push(Stmt { span, kind: StmtKind::Nregs(value) });
            }
            _ => {
                let mnemonic = self.ident("an instruction")?;
                let const_at = const_operand_index(&mnemonic.name);
                let operands = if self.at_eol() || self.brace_ends_stmt() {
                    Vec::new()
                } else {
                    self.operands_to_eol(|k| Some(k) == const_at)?
                };
                let span = mnemonic.span.join(self.prev);
                self.eat_newline();
                let inst = Inst {
                    span,
                    mnemonic: mnemonic.name,
                    mnemonic_span: mnemonic.span,
                    operands,
                };
                body.push(Stmt { span, kind: StmtKind::Inst(inst) });
            }
        }
        Ok(())
    }

    // ---- types --------------------------------------------------------------------------

    fn parse_type(&mut self) -> PResult<TypeRef> {
        if let TokenKind::Name(n) = self.kind() {
            if let Some(ty) = Ty::from_name(n) {
                let span = self.bump();
                return Ok(TypeRef { ty, span });
            }
        }
        Err(self.expected("a type (`i32`, `i64`, `f32` or `f64`)"))
    }

    /// `(` [type { `,` type }] `)`.
    fn type_list_paren(&mut self, what: &str) -> PResult<Vec<TypeRef>> {
        self.expect_tok(TokenKind::LParen, what)?;
        let mut types = Vec::new();
        if matches!(self.kind(), TokenKind::RParen) {
            self.bump();
            return Ok(types);
        }
        loop {
            types.push(self.parse_type()?);
            match self.kind() {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::RParen => {
                    self.bump();
                    return Ok(types);
                }
                _ => return Err(self.expected("`,` or `)` after the type")),
            }
        }
    }

    /// `[ -> type ]` or `[ -> ( type { , type } ) ]` of a function or import header.
    fn opt_results(&mut self) -> PResult<Vec<TypeRef>> {
        if !matches!(self.kind(), TokenKind::Arrow) {
            return Ok(Vec::new());
        }
        self.bump();
        if matches!(self.kind(), TokenKind::LParen) {
            let open = self.tok().span;
            let list = self.type_list_paren("`(`")?;
            if list.is_empty() {
                self.push_diag(
                    open.join(self.prev),
                    "`-> ()` is not allowed; leave out the arrow when there are no results".to_string(),
                    None,
                );
            }
            Ok(list)
        } else {
            Ok(vec![self.parse_type()?])
        }
    }

    // ---- operands -----------------------------------------------------------------------

    /// One or more operands separated by commas, up to the end of the line (not consumed).
    /// `is_const(k)` says that operand `k` (0-based) is a constant-expression position.
    fn operands_to_eol(&mut self, is_const: impl Fn(usize) -> bool) -> PResult<Vec<Operand>> {
        let mut ops = Vec::new();
        loop {
            let op = if is_const(ops.len()) { self.const_operand()? } else { self.parse_operand()? };
            ops.push(op);
            if matches!(self.kind(), TokenKind::Comma) {
                self.bump();
                continue;
            }
            if self.at_eol() || self.brace_ends_stmt() {
                return Ok(ops);
            }
            return Err(self.expected("`,` or end of line after operand"));
        }
    }

    /// An operand in a constant-expression position (efa-syntax 18): a literal or name as
    /// usual, or an expression with `+ - * / % << >> & | ^ ~` and parentheses, parsed by
    /// `calc`. The expression ends at the first token that cannot continue it (`,`, end of
    /// line, `]`, an operand that follows an operand).
    fn const_operand(&mut self) -> PResult<Operand> {
        let first = self.kind();
        let starts = matches!(
            first,
            TokenKind::Name(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::Char(_)
                | TokenKind::LParen
                | TokenKind::Minus
                | TokenKind::Plus
                | TokenKind::Tilde
        );
        let single = matches!(
            first,
            TokenKind::Name(_) | TokenKind::Int(_) | TokenKind::Float(_) | TokenKind::Char(_)
        );
        if !starts || (single && !continues_expr(&self.tok_at(1).kind)) {
            return self.parse_operand();
        }
        self.expression()
    }

    fn expression(&mut self) -> PResult<Operand> {
        use calc::TokenKind as C;
        let start = self.pos;
        let mut toks: Vec<calc::Token> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        // The efa token index each calc token came from.
        let mut origin: Vec<usize> = Vec::new();
        let mut depth = 0usize;
        let mut after_operand = false;
        let mut i = start;
        while let Some(t) = self.toks.get(i) {
            let span = t.span;
            // A literal with a leading `-` right after an operand is a subtraction.
            let split = |span: Span| {
                (
                    Span::new(span.line, span.column, 1),
                    Span::new(span.line, span.column + 1, span.len.saturating_sub(1).max(1)),
                )
            };
            let mut kinds: Vec<(C, Span)> = Vec::new();
            match &t.kind {
                TokenKind::Name(n) => kinds.push((C::Ident(n.clone()), span)),
                TokenKind::Int(v) => match v.checked_neg().filter(|_| *v < 0 && after_operand) {
                    Some(pos) => {
                        let (m, n) = split(span);
                        kinds.push((C::Minus, m));
                        kinds.push((C::Int(pos), n));
                    }
                    None => kinds.push((C::Int(*v), span)),
                },
                TokenKind::Char(c) => kinds.push((C::Int(*c as i128), span)),
                TokenKind::Float(text) => {
                    let (neg, body) = match text.strip_prefix('-') {
                        Some(b) => (true, b),
                        None => (false, text.as_str()),
                    };
                    let x = if body == "inf" {
                        f64::INFINITY
                    } else if body == "nan" {
                        f64::NAN
                    } else if body.starts_with("nan:") {
                        return Err(self.error(
                            span,
                            "`nan:0x...` (a NaN with a payload) cannot be part of a constant expression",
                        ));
                    } else {
                        match body.parse::<f64>() {
                            Ok(x) => x,
                            Err(_) => return Err(self.error(span, format!("invalid float literal `{text}`"))),
                        }
                    };
                    if neg && after_operand {
                        let (m, n) = split(span);
                        kinds.push((C::Minus, m));
                        kinds.push((C::Float(x), n));
                    } else {
                        kinds.push((C::Float(if neg { -x } else { x }), span));
                    }
                }
                TokenKind::LParen => {
                    depth += 1;
                    kinds.push((C::LParen, span));
                }
                TokenKind::RParen => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    kinds.push((C::RParen, span));
                }
                TokenKind::Plus => kinds.push((C::Plus, span)),
                TokenKind::Minus => kinds.push((C::Minus, span)),
                TokenKind::Star => kinds.push((C::Star, span)),
                TokenKind::Slash => kinds.push((C::Slash, span)),
                TokenKind::Percent => kinds.push((C::Percent, span)),
                TokenKind::Amp => kinds.push((C::Amp, span)),
                TokenKind::Pipe => kinds.push((C::Pipe, span)),
                TokenKind::Caret => kinds.push((C::Caret, span)),
                TokenKind::Tilde => kinds.push((C::Tilde, span)),
                TokenKind::Shl => kinds.push((C::Shl, span)),
                TokenKind::Shr => kinds.push((C::Shr, span)),
                _ => break,
            }
            after_operand = matches!(
                t.kind,
                TokenKind::Name(_)
                    | TokenKind::Int(_)
                    | TokenKind::Float(_)
                    | TokenKind::Char(_)
                    | TokenKind::RParen
            );
            for (k, sp) in kinds {
                let at = toks.len();
                toks.push(calc::Token::new(k, at, at + 1));
                spans.push(sp);
                origin.push(i);
            }
            i += 1;
        }
        let total = toks.len();
        if total > MAX_EXPR_TOKENS {
            let span = spans.get(MAX_EXPR_TOKENS).copied().unwrap_or_else(|| self.tok().span);
            return Err(self.error(
                span,
                format!("the constant expression is too long (at most {MAX_EXPR_TOKENS} tokens)"),
            ));
        }
        let mut src = calc::TokenIter(toks.into_iter());
        let mut p = calc::Parser::new(&mut src);
        let parsed = p.parse_expr().and_then(|e| {
            let used = p.peek()?.map_or(total, |t| t.span.start);
            Ok((e, used))
        });
        let (expr, used) = match parsed {
            Ok(x) => x,
            Err(e) => {
                let at = |k: usize| spans.get(k).copied().unwrap_or_else(|| self.tok().span);
                let span = match e.kind {
                    calc::CalcErrorKind::UnexpectedEnd => at(total.saturating_sub(1)),
                    _ => at(e.span.start).join(at(e.span.end.saturating_sub(1).max(e.span.start))),
                };
                let what = match e.kind {
                    calc::CalcErrorKind::UnexpectedEnd => "the constant expression is incomplete".to_string(),
                    calc::CalcErrorKind::UnclosedBracket => "unclosed `(` in a constant expression".to_string(),
                    k => format!("invalid constant expression: {k}"),
                };
                return Err(self.error(span, what));
            }
        };
        // Resume after the last token the expression used.
        self.pos = origin.get(used).copied().unwrap_or(i);
        let last = spans.get(used.saturating_sub(1)).copied().unwrap_or_else(Span::none);
        self.prev = last;
        let span = spans.first().copied().unwrap_or(last).join(last);
        Ok(Operand::Expr(Box::new(ConstExpr { span, expr, spans })))
    }

    fn parse_operand(&mut self) -> PResult<Operand> {
        if self.depth >= MAX_DEPTH {
            let span = self.tok().span;
            return Err(self.error(span, "operand is nested too deeply"));
        }
        self.depth += 1;
        let r = self.operand_inner();
        self.depth -= 1;
        r
    }

    fn operand_inner(&mut self) -> PResult<Operand> {
        let tok = self.tok();
        let callable = matches!(self.tok_at(1).kind, TokenKind::LParen);
        match &tok.kind {
            TokenKind::Name(n) => {
                self.bump();
                let op = Operand::Name(Ident { name: n.clone(), span: tok.span });
                if callable {
                    self.call_sugar(op)
                } else {
                    Ok(op)
                }
            }
            TokenKind::Int(v) => {
                self.bump();
                let op = Operand::Int(*v, tok.span);
                if callable {
                    self.call_sugar(op)
                } else {
                    Ok(op)
                }
            }
            TokenKind::Char(c) => {
                self.bump();
                let op = Operand::Char(*c, tok.span);
                if callable {
                    self.call_sugar(op)
                } else {
                    Ok(op)
                }
            }
            TokenKind::Float(s) => {
                self.bump();
                Ok(Operand::Float(s.clone(), tok.span))
            }
            TokenKind::Str(b) => {
                self.bump();
                Ok(Operand::Str(b.clone(), tok.span))
            }
            TokenKind::LBracket => self.label_list(),
            TokenKind::LParen => {
                if self.group_is_signature() {
                    self.signature()
                } else {
                    self.tuple()
                }
            }
            _ => Err(self.expected("an operand")),
        }
    }

    /// At a `(`: is the matching `)` followed by `->`? (Stops at the end of the line.)
    fn group_is_signature(&self) -> bool {
        let mut depth = 0usize;
        let mut i = 0usize;
        loop {
            match &self.tok_at(i).kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return matches!(self.tok_at(i + 1).kind, TokenKind::Arrow);
                    }
                }
                TokenKind::Newline | TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
            if self.pos + i >= self.toks.len() {
                return false;
            }
        }
    }

    /// After a consumed `(`: operands up to and including the `)`.
    fn operands_after_lparen(&mut self, what: &str) -> PResult<Vec<Operand>> {
        let mut ops = Vec::new();
        if matches!(self.kind(), TokenKind::RParen) {
            self.bump();
            return Ok(ops);
        }
        loop {
            ops.push(self.parse_operand()?);
            match self.kind() {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::RParen => {
                    self.bump();
                    return Ok(ops);
                }
                _ => return Err(self.expected(&format!("`,` or `)` after {}", what))),
            }
        }
    }

    /// At the `(` after the callee: `(args) [-> result | -> (results)]`.
    fn call_sugar(&mut self, callee: Operand) -> PResult<Operand> {
        let start = callee.span();
        self.bump(); // `(`
        let args = self.operands_after_lparen("the argument")?;
        let mut results = Vec::new();
        if matches!(self.kind(), TokenKind::Arrow) {
            self.bump();
            if matches!(self.kind(), TokenKind::LParen) {
                self.bump();
                results = self.operands_after_lparen("the result")?;
            } else {
                results.push(self.parse_operand()?);
            }
        }
        let span = start.join(self.prev);
        Ok(Operand::Call(Box::new(CallSugar { span, callee, args, results })))
    }

    fn tuple(&mut self) -> PResult<Operand> {
        let start = self.bump(); // `(`
        let items = self.operands_after_lparen("the tuple element")?;
        Ok(Operand::Tuple(items, start.join(self.prev)))
    }

    fn signature(&mut self) -> PResult<Operand> {
        let start = self.tok().span;
        let params = self.type_list_paren("`(`")?;
        self.expect_tok(TokenKind::Arrow, "`->` after the signature parameters")?;
        let results = if matches!(self.kind(), TokenKind::LParen) {
            self.type_list_paren("`(`")?
        } else {
            vec![self.parse_type()?]
        };
        Ok(Operand::Signature(Signature { span: start.join(self.prev), params, results }))
    }

    fn label_list(&mut self) -> PResult<Operand> {
        let start = self.bump(); // `[`
        let mut labels = Vec::new();
        if matches!(self.kind(), TokenKind::RBracket) {
            self.bump();
            return Ok(Operand::Labels(labels, start.join(self.prev)));
        }
        loop {
            labels.push(self.ident("a label name")?);
            match self.kind() {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::RBracket => {
                    self.bump();
                    return Ok(Operand::Labels(labels, start.join(self.prev)));
                }
                _ => return Err(self.expected("`,` or `]` after the label")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::lexer::lex;
    use crate::asm::{Diagnostics, parse as parse_src};

    const TOUR: &str = r##"; tour.efa: exercises the whole language
import std.println(i64, i64)
import std.print_i64(i64)
import std.print_f64(f64)
import std.print_char(i32)

memory 1 4
const LIMIT = 10
const PI = 3.14159
const NEWLINE = '\n'

global counter: i64 = 0
global mut total: f64 = 1.5
export global mut ticks: i32 = -1

table [inc, null, twice]

msg    db "hello world!", NEWLINE, 0
words  dw 1, 2, 0xffff
floats dd 1.5, nan:0x7fc00001
reals  dq 2.5, -0.0, inf
buf    resb 64

fn inc(x: i64) -> i64 {
    i64.add x, x, 1                 ; immediate source
    ret x
}

fn twice(x: i64) -> i64 {
    i64.add x, x, x
    ret x
}

fn swap(a: i64, b: i64) -> (i64, i64) {
    ret (b, a)                      ; window copy
}

fn half(x: f64) -> f64 {
    f64.div x, x, 2                 ; integer literal in a float slot
    ret x
}

fn raw_demo(a: i64, r1: i64) -> i64 {      ; r1 is an unnamed parameter
    reg idx
    const.i32 idx, 0
    mov r4, a                       ; window for the raw calls, base r4
    call_indirect (i64) -> i64, idx, r4
    call inc, r4
    ret r4
}

fn never() {
    unreachable
}

export fn main() -> i32 {
    reg i, acc, t, f, x, y, h
    nregs 12
    const.i64 acc, 0
    const.i64 i, LIMIT
loop:
    i64.eqz t, i
    br_if t, done
    i64.add acc, acc, i
    i64.sub i, i, 1
    jmp loop
done:
    call std.print_i64(acc)
    call std.print_char(NEWLINE)
    call std.println(msg, msg.len)
    call swap(acc, 7) -> (x, y)
    call inc(x) -> x
    call_indirect (i64) -> i64, 2(acc) -> y       ; immediate index: twice
    const.i32 t, 0
    call_indirect (i64) -> i64, t(acc) -> y       ; register index: inc
    call raw_demo(acc, 0) -> x
    i64.store 42, buf, 8            ; immediate source and base symbol
    i64.load x, buf, 8
    i32.store8 'A', buf, 1
    i32.load8_u t, buf, 1
    i64.load y, buf                 ; offset omitted
    f64.load f, reals, 8
    f64.add f, f, PI
    call half(f) -> f
    call std.print_f64(f)
    const.f32 h, 1.5
    f32.add h, h, 2
    f64.promote_f32 f, h
    i64.trunc_f64_s x, f
    i32.wrap_i64 t, x
    select x, t, 1, 2
    global.get y, counter
    i64.add y, y, 1
    global.set counter, y
    global.set total, 2.5
    global.set ticks, -5
    memory.size x
    memory.grow y, 1
    memory.fill buf, 0, 16
    memory.copy buf, msg, msg.len
    nop
    yield
    i32.wrap_i64 t, acc
    br_table t, [first, second], other
first:
    call std.print_char('1')
    jmp finish
second:
    call std.print_char('2')
    jmp finish
other:
    call std.print_char('?')
finish:
    ret 0
}
"##;

    fn ok(src: &str) -> Module {
        match parse_src(src, "t.efa") {
            Ok(m) => m,
            Err(d) => panic!("unexpected diagnostics:\n{}", d),
        }
    }

    fn errs(src: &str) -> Vec<Diagnostic> {
        match parse_src(src, "t.efa") {
            Ok(m) => panic!("expected errors, parsed {:?}", m),
            Err(Diagnostics(d)) => d,
        }
    }

    /// Asserts that exactly one diagnostic exists, with the message part at line:column.
    fn one_err(src: &str, msg: &str, line: u32, col: u32) -> Diagnostic {
        let d = errs(src);
        assert_eq!(d.len(), 1, "diagnostics: {:?}", d);
        let e = d.into_iter().next().unwrap();
        assert!(e.message.contains(msg), "message {:?} lacks {:?}", e.message, msg);
        assert_eq!((e.line, e.column), (line, col), "position of {:?}", e.message);
        e
    }

    fn func(m: &Module, i: usize) -> &FuncDecl {
        match &m.items[i] {
            Item::Func(f) => f,
            other => panic!("not a function: {:?}", other),
        }
    }

    /// Parses the statements of `fn f() {` + lines + `}`; statement lines start at line 2.
    fn body(lines: &str) -> Vec<Stmt> {
        let m = ok(&format!("fn f() {{\n{}\n}}\n", lines));
        func(&m, 0).body.clone()
    }

    fn inst(line: &str) -> Inst {
        let mut b = body(line);
        match b.remove(0).kind {
            StmtKind::Inst(i) => i,
            other => panic!("not an instruction: {:?}", other),
        }
    }

    fn name(op: &Operand) -> &str {
        match op {
            Operand::Name(i) => &i.name,
            other => panic!("not a name: {:?}", other),
        }
    }

    // ---- items --------------------------------------------------------------------------

    #[test]
    fn empty_module() {
        assert!(ok("").items.is_empty());
        assert!(ok("\n; comment\n\n").items.is_empty());
    }

    #[test]
    fn import_item() {
        let m = ok("import std.println(i64, i64)\nimport a.b.c(f32) -> f64\nimport x.y() -> (i32, i64)\nimport x.z()\n");
        assert_eq!(m.items.len(), 4);
        let Item::Import(i) = &m.items[0] else { panic!() };
        assert_eq!(i.qualified.name, "std.println");
        assert_eq!(i.split(), Some(("std", "println")));
        assert_eq!(i.params.iter().map(|t| t.ty).collect::<Vec<_>>(), vec![Ty::I64, Ty::I64]);
        assert!(i.results.is_empty());
        assert_eq!(i.span, Span::new(1, 1, 28));
        assert_eq!(i.qualified.span, Span::new(1, 8, 11));
        let Item::Import(i) = &m.items[1] else { panic!() };
        assert_eq!(i.split(), Some(("a.b", "c")));
        assert_eq!(i.results[0].ty, Ty::F64);
        assert_eq!(i.results[0].span, Span::new(2, 22, 3));
        let Item::Import(i) = &m.items[2] else { panic!() };
        assert!(i.params.is_empty());
        assert_eq!(i.results.len(), 2);
        let Item::Import(i) = &m.items[3] else { panic!() };
        assert!(i.params.is_empty() && i.results.is_empty());
    }

    #[test]
    fn memory_item() {
        let m = ok("memory 1\nmemory 2 PAGES\nmemory 0x10 64\n");
        let Item::Memory(a) = &m.items[0] else { panic!() };
        assert_eq!(a.min, Operand::Int(1, Span::new(1, 8, 1)));
        assert!(a.max.is_none());
        assert_eq!(a.span, Span::new(1, 1, 8));
        let Item::Memory(b) = &m.items[1] else { panic!() };
        assert_eq!(name(b.max.as_ref().unwrap()), "PAGES");
        let Item::Memory(c) = &m.items[2] else { panic!() };
        assert_eq!(c.min, Operand::Int(16, Span::new(3, 8, 4)));
    }

    #[test]
    fn const_item() {
        let m = ok("const A = 123\nconst B = -1.5e3\nconst C = 'x'\nconst D = nan\n");
        let vals: Vec<&Operand> = m
            .items
            .iter()
            .map(|i| match i {
                Item::Const(c) => &c.value,
                _ => panic!(),
            })
            .collect();
        assert!(matches!(vals[0], Operand::Int(123, _)));
        assert!(matches!(vals[1], Operand::Float(s, _) if s == "-1.5e3"));
        assert!(matches!(vals[2], Operand::Char('x', _)));
        assert!(matches!(vals[3], Operand::Float(s, _) if s == "nan"));
        let Item::Const(c) = &m.items[0] else { panic!() };
        assert_eq!(c.name.name, "A");
        assert_eq!(c.name.span, Span::new(1, 7, 1));
        assert_eq!(c.span, Span::new(1, 1, 13));
    }

    #[test]
    fn global_items() {
        let m = ok("global a: i64 = 0\nglobal mut b: f64 = 1.5\nexport global c: i32 = -1\nexport global mut d: f32 = inf\nglobal e: i64 = msg.len\n");
        let g = |i: usize| match &m.items[i] {
            Item::Global(g) => g.clone(),
            _ => panic!(),
        };
        assert!(!g(0).exported && !g(0).mutable);
        assert!(!g(1).exported && g(1).mutable);
        assert!(g(2).exported && !g(2).mutable);
        assert!(g(3).exported && g(3).mutable);
        assert_eq!(g(1).ty.ty, Ty::F64);
        assert_eq!(g(3).name.name, "d");
        assert_eq!(g(2).span, Span::new(3, 1, 25));
        assert_eq!(name(&g(4).init), "msg.len");
    }

    #[test]
    fn global_named_mut_is_not_the_keyword() {
        let m = ok("global mut: i64 = 0\n");
        let Item::Global(g) = &m.items[0] else { panic!() };
        assert!(!g.mutable);
        assert_eq!(g.name.name, "mut");
    }

    #[test]
    fn table_item() {
        let m = ok("table [f1, null, std.println]\ntable []\ntable [only]\n");
        let Item::Table(t) = &m.items[0] else { panic!() };
        assert_eq!(t.entries.len(), 3);
        assert!(matches!(&t.entries[0], TableEntry::Func(i) if i.name == "f1"));
        assert!(matches!(&t.entries[1], TableEntry::Null(s) if *s == Span::new(1, 12, 4)));
        assert!(matches!(&t.entries[2], TableEntry::Func(i) if i.name == "std.println"));
        let Item::Table(t) = &m.items[1] else { panic!() };
        assert!(t.entries.is_empty());
    }

    #[test]
    fn data_items() {
        let m = ok("a db \"hi\", 10, 0xff, 'c', K\nb dw 1, 2\nc dd 1, 2.5\nd dq 1, -0.5\ne resb 64\n");
        let d = |i: usize| match &m.items[i] {
            Item::Data(d) => d.clone(),
            _ => panic!(),
        };
        assert_eq!(d(0).kind, DataKind::Db);
        assert_eq!(d(0).values.len(), 5);
        assert!(matches!(&d(0).values[0], Operand::Str(b, _) if b == b"hi"));
        assert!(matches!(&d(0).values[3], Operand::Char('c', _)));
        assert_eq!(name(&d(0).values[4]), "K");
        assert_eq!(d(0).kind_span, Span::new(1, 3, 2));
        assert_eq!(d(0).name.span, Span::new(1, 1, 1));
        assert_eq!(d(0).span, Span::new(1, 1, 27));
        assert_eq!(d(1).kind, DataKind::Dw);
        assert_eq!(d(2).kind, DataKind::Dd);
        assert!(matches!(&d(2).values[1], Operand::Float(s, _) if s == "2.5"));
        assert_eq!(d(3).kind, DataKind::Dq);
        assert_eq!(d(4).kind, DataKind::Resb);
        assert_eq!(d(4).values, vec![Operand::Int(64, Span::new(5, 8, 2))]);
    }

    #[test]
    fn data_line_continuation_after_comma() {
        let m = ok("big db 1, 2,\n    3, 4, ; c\n\n    \"x\"\nnext dw 1\n");
        assert_eq!(m.items.len(), 2);
        let Item::Data(d) = &m.items[0] else { panic!() };
        assert_eq!(d.values.len(), 5);
        let Item::Data(d) = &m.items[1] else { panic!() };
        assert_eq!(d.name.name, "next");
    }

    #[test]
    fn function_headers() {
        let m = ok("fn a() {\n nop\n}\nexport fn b(x: i64, y: f32) -> i32 {\n ret\n}\nfn c(r0: i64) -> (i64, f64) {\n ret\n}\n");
        let a = func(&m, 0);
        assert!(!a.exported && a.params.is_empty() && a.results.is_empty());
        assert_eq!(a.span, Span::new(1, 1, 8));
        assert_eq!(a.end, Span::new(3, 1, 1));
        let b = func(&m, 1);
        assert!(b.exported);
        assert_eq!(b.name.name, "b");
        assert_eq!(b.params.len(), 2);
        assert_eq!(b.params[1].name.name, "y");
        assert_eq!(b.params[1].ty.ty, Ty::F32);
        assert_eq!(b.results.len(), 1);
        assert_eq!(b.span, Span::new(4, 1, 36));
        let c = func(&m, 2);
        assert_eq!(c.params[0].name.name, "r0");
        assert_eq!(c.results.iter().map(|t| t.ty).collect::<Vec<_>>(), vec![Ty::I64, Ty::F64]);
    }

    #[test]
    fn items_in_any_order_and_blank_lines() {
        let m = ok("\n\nfn f() {\n ret\n}\n\n\nmemory 1\nx db 1\nimport a.b()\nconst K = 1\n");
        assert_eq!(m.items.len(), 5);
        assert!(matches!(m.items[0], Item::Func(_)));
        assert!(matches!(m.items[4], Item::Const(_)));
        assert_eq!(m.items[1].span().line, 8);
    }

    #[test]
    fn crlf_source() {
        let m = ok("fn f() {\r\n    ret\r\n}\r\n");
        assert_eq!(func(&m, 0).body.len(), 1);
    }

    // ---- statements ---------------------------------------------------------------------

    #[test]
    fn labels_and_statements() {
        let b = body("loop:\n  nop\na: b: ret\ndone: ret acc\nend:");
        let kinds: Vec<String> = b
            .iter()
            .map(|s| match &s.kind {
                StmtKind::Label(i) => format!("label {}", i.name),
                StmtKind::Inst(i) => format!("inst {}", i.mnemonic),
                _ => "other".to_string(),
            })
            .collect();
        assert_eq!(
            kinds,
            ["label loop", "inst nop", "label a", "label b", "inst ret", "label done", "inst ret", "label end"]
        );
        assert_eq!(b[0].span, Span::new(2, 1, 5));
        assert_eq!(b[2].span, Span::new(4, 1, 2));
        assert_eq!(b[3].span, Span::new(4, 4, 2));
    }

    #[test]
    fn reg_and_nregs() {
        let b = body("reg a, b, c\nreg d\nnregs 12\nnregs N");
        let StmtKind::Reg(r) = &b[0].kind else { panic!() };
        assert_eq!(r.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert_eq!(b[0].span, Span::new(2, 1, 11));
        let StmtKind::Reg(r) = &b[1].kind else { panic!() };
        assert_eq!(r.len(), 1);
        assert!(matches!(&b[2].kind, StmtKind::Nregs(Operand::Int(12, _))));
        assert_eq!(b[2].span, Span::new(4, 1, 8));
        assert!(matches!(&b[3].kind, StmtKind::Nregs(Operand::Name(i)) if i.name == "N"));
    }

    #[test]
    fn simple_operand_forms() {
        let i = inst("i64.add d, 5, -7");
        assert_eq!(i.mnemonic, "i64.add");
        assert_eq!(i.mnemonic_span, Span::new(2, 1, 7));
        assert_eq!(i.span, Span::new(2, 1, 16));
        assert_eq!(name(&i.operands[0]), "d");
        assert_eq!(i.operands[1], Operand::Int(5, Span::new(2, 12, 1)));
        assert_eq!(i.operands[2], Operand::Int(-7, Span::new(2, 15, 2)));

        let i = inst("  const.f64 x, 1.5e3");
        assert_eq!(i.mnemonic_span.column, 3);
        assert!(matches!(&i.operands[1], Operand::Float(s, _) if s == "1.5e3"));
        for (text, want) in [("inf", "inf"), ("-inf", "-inf"), ("nan", "nan"), ("nan:0x7fc00001", "nan:0x7fc00001")] {
            let i = inst(&format!("const.f32 x, {}", text));
            assert!(matches!(&i.operands[1], Operand::Float(s, _) if s == want), "{}", text);
        }
        let i = inst("i32.store8 'A', buf, 1");
        assert!(matches!(i.operands[0], Operand::Char('A', _)));
        let i = inst("nop");
        assert!(i.operands.is_empty());
        let i = inst("global.get y, msg.len");
        assert_eq!(name(&i.operands[1]), "msg.len");
        let i = inst("i64.add a, b, c ; trailing comment");
        assert_eq!(i.operands.len(), 3);
        let i = inst("i64.add a,\n    b,\n    c");
        assert_eq!(i.operands.len(), 3);
    }

    #[test]
    fn call_sugar_forms() {
        let i = inst("call f(a, 7) -> (x, y)");
        assert_eq!(name(&i.operands[0].clone().into_call_callee()), "f");
        let Operand::Call(c) = &i.operands[0] else { panic!() };
        assert_eq!(c.args.len(), 2);
        assert_eq!(c.results.len(), 2);
        assert_eq!(c.span, Span::new(2, 6, 17));
        assert_eq!(i.operands.len(), 1);

        let Operand::Call(c) = &inst("call f(a) -> x").operands[0] else { panic!() };
        assert_eq!(c.results.len(), 1);
        let Operand::Call(c) = &inst("call std.println(msg, msg.len)").operands[0] else { panic!() };
        assert_eq!(name(&c.callee), "std.println");
        assert!(c.results.is_empty());
        let Operand::Call(c) = &inst("call f()").operands[0] else { panic!() };
        assert!(c.args.is_empty() && c.results.is_empty());
        let Operand::Call(c) = &inst("call f () -> ()").operands[0] else { panic!() };
        assert!(c.results.is_empty());
        let Operand::Call(c) = &inst("call f(a,\n b) -> x").operands[0] else { panic!() };
        assert_eq!(c.args.len(), 2);
    }

    // Small helper so the test above can read the callee of the first operand.
    trait IntoCallee {
        fn into_call_callee(self) -> Operand;
    }
    impl IntoCallee for Operand {
        fn into_call_callee(self) -> Operand {
            match self {
                Operand::Call(c) => c.callee,
                other => other,
            }
        }
    }

    #[test]
    fn call_indirect_forms() {
        let i = inst("call_indirect (i64, i64) -> i64, idx(a, b) -> x");
        let Operand::Signature(s) = &i.operands[0] else { panic!("{:?}", i.operands) };
        assert_eq!(s.params.len(), 2);
        assert_eq!(s.results.len(), 1);
        assert_eq!(s.span, Span::new(2, 15, 17));
        let Operand::Call(c) = &i.operands[1] else { panic!() };
        assert_eq!(name(&c.callee), "idx");
        assert_eq!(c.results.len(), 1);

        let i = inst("call_indirect (i64) -> i64, 2(acc) -> y");
        let Operand::Call(c) = &i.operands[1] else { panic!() };
        assert!(matches!(c.callee, Operand::Int(2, _)));
        let i = inst("call_indirect (i64) -> (), 'a'(acc)");
        let Operand::Signature(s) = &i.operands[0] else { panic!() };
        assert!(s.results.is_empty());
        let Operand::Call(c) = &i.operands[1] else { panic!() };
        assert!(matches!(c.callee, Operand::Char('a', _)));

        let i = inst("call_indirect () -> (i64, f32), idx, r4");
        let Operand::Signature(s) = &i.operands[0] else { panic!() };
        assert!(s.params.is_empty());
        assert_eq!(s.results.iter().map(|t| t.ty).collect::<Vec<_>>(), vec![Ty::I64, Ty::F32]);
        assert_eq!(i.operands.len(), 3);
        assert_eq!(name(&i.operands[2]), "r4");

        // without `->` it is a tuple, not a signature
        let i = inst("call_indirect (i64), idx, r4");
        assert!(matches!(&i.operands[0], Operand::Tuple(t, _) if t.len() == 1));
    }

    #[test]
    fn raw_call_and_ret_forms() {
        let i = inst("call f, r5");
        assert_eq!(i.operands.len(), 2);
        assert_eq!(name(&i.operands[1]), "r5");
        let i = inst("ret");
        assert!(i.operands.is_empty());
        let i = inst("ret a");
        assert_eq!(name(&i.operands[0]), "a");
        let i = inst("ret 0");
        assert!(matches!(i.operands[0], Operand::Int(0, _)));
        let i = inst("ret (a, 5, 'x')");
        let Operand::Tuple(t, s) = &i.operands[0] else { panic!() };
        assert_eq!(t.len(), 3);
        assert_eq!(*s, Span::new(2, 5, 11));
        let i = inst("ret ()");
        assert!(matches!(&i.operands[0], Operand::Tuple(t, _) if t.is_empty()));
        let i = inst("ret (a)");
        assert!(matches!(&i.operands[0], Operand::Tuple(t, _) if t.len() == 1));
    }

    #[test]
    fn br_table_label_list() {
        let i = inst("br_table idx, [a, b, c], default");
        let Operand::Labels(l, s) = &i.operands[1] else { panic!() };
        assert_eq!(l.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert_eq!(*s, Span::new(2, 15, 9));
        assert_eq!(name(&i.operands[2]), "default");
        let i = inst("br_table idx, [], d");
        assert!(matches!(&i.operands[1], Operand::Labels(l, _) if l.is_empty()));
        let i = inst("br_table idx, [a,\n b], d");
        assert!(matches!(&i.operands[1], Operand::Labels(l, _) if l.len() == 2));
    }

    #[test]
    fn nested_and_odd_operands_parse_syntactically() {
        // The parser accepts call sugar anywhere; the lowering rejects what has no meaning.
        let i = inst("i64.add a, f(g(1), h()) -> x, 3");
        assert_eq!(i.operands.len(), 3);
        assert!(matches!(&i.operands[1], Operand::Call(_)));
        let i = inst("mov a, \"str\"");
        assert!(matches!(&i.operands[1], Operand::Str(b, _) if b == b"str"));
        // any name, even a keyword, is a mnemonic word here (the lowering decides)
        let i = inst("frobnicate a");
        assert_eq!(i.mnemonic, "frobnicate");
    }

    // ---- the design's examples ----------------------------------------------------------

    #[test]
    fn design_section_1_hello() {
        let m = ok("; hello.efa\nimport std.println(i64, i64)          ; pointer, length\n\nmsg db \"hello world!\"\n\nexport fn main() {\n    call std.println(msg, msg.len)\n    ret\n}\n");
        assert_eq!(m.items.len(), 3);
        let f = func(&m, 2);
        assert!(f.exported);
        assert_eq!(f.body.len(), 2);
    }

    #[test]
    fn design_section_4_sum() {
        let m = ok("export fn sum(n: i64) -> i64 {\n    reg acc, t                    ; named registers (after the parameters)\n    const.i64 acc, 0\nloop:\n    i64.eqz t, n\n    br_if t, done\n    i64.add acc, acc, n\n    i64.sub n, n, 1               ; immediate in a source position (section 5)\n    jmp loop\ndone:\n    ret acc\n}\n");
        let f = func(&m, 0);
        assert_eq!(f.body.len(), 10);
        assert_eq!(f.name.name, "sum");
    }

    #[test]
    fn design_section_3_items() {
        ok("import ns.name(i64, i64) -> i64\nimport ns.two(i64) -> (i64, i64)\nmemory 1 2\nconst NAME = 123\nglobal NAME2: i64 = 0\nglobal mut NAME3: f64 = 1.5\ntable [f1, f2, f3]\nNAME4 db \"text\", 10, 0xff\nNAME5 dw 1, 2\nNAME6 dd 1, 2\nNAME7 dq 1, 2\nNAME8 resb 64\nfn NAME9(a: i64) -> i64 {\n ret a\n}\n");
    }

    #[test]
    fn section_15_complete_example() {
        let m = ok(TOUR);
        assert_eq!(m.items.len(), 24);
        let imports = m.items.iter().filter(|i| matches!(i, Item::Import(_))).count();
        let funcs: Vec<&FuncDecl> = m
            .items
            .iter()
            .filter_map(|i| if let Item::Func(f) = i { Some(f) } else { None })
            .collect();
        assert_eq!(imports, 4);
        assert_eq!(funcs.len(), 7);
        let names: Vec<&str> = funcs.iter().map(|f| f.name.name.as_str()).collect();
        assert_eq!(names, ["inc", "twice", "swap", "half", "raw_demo", "never", "main"]);
        let main = funcs[6];
        assert!(main.exported);
        assert_eq!(main.results[0].ty, Ty::I32);
        assert_eq!(main.body.len(), 58);
        let labels = main.body.iter().filter(|s| matches!(s.kind, StmtKind::Label(_))).count();
        assert_eq!(labels, 6);
        assert!(matches!(&main.body[0].kind, StmtKind::Reg(r) if r.len() == 7));
        assert!(matches!(&main.body[1].kind, StmtKind::Nregs(Operand::Int(12, _))));
        let raw = funcs[4];
        assert_eq!(raw.params[1].name.name, "r1");
        // every statement span is on a line inside the body
        for s in &main.body {
            assert!(s.span.line > main.span.line && s.span.line < main.end.line);
        }
    }

    // ---- errors -------------------------------------------------------------------------

    #[test]
    fn err_missing_comma_between_operands() {
        // `i64.add a b`: `b` is at column 15
        one_err("fn f() {\n    i64.add a b\n    ret\n}\n", "expected `,` or end of line after operand, found `b`", 2, 15);
    }

    #[test]
    fn err_brace_after_operand() {
        one_err(
            "fn f() {\n    i64.add a, b {\n    ret\n}\n",
            "expected `,` or end of line after operand, found `{`",
            2,
            18,
        );
    }

    #[test]
    fn err_missing_operand_after_comma() {
        // the lexer drops the newline after `,`, so the next line is where the operand is
        // expected; here the file ends after the comma
        let d = errs("fn f() {\n    i64.add a,");
        assert!(d.iter().any(|e| e.message.contains("expected an operand, found end of line")), "{:?}", d);
    }

    #[test]
    fn err_unclosed_paren_in_call() {
        one_err("fn f() {\n    call g(a, b\n    ret\n}\n", "expected `,` or `)` after the argument, found end of line", 2, 16);
    }

    #[test]
    fn err_bad_operand_token() {
        one_err("fn f() {\n    mov a, )\n    ret\n}\n", "expected an operand, found `)`", 2, 12);
        one_err("fn f() {\n    mov a, ->\n    ret\n}\n", "expected an operand, found `->`", 2, 12);
    }

    #[test]
    fn err_const_without_equals() {
        one_err("const X 5\n", "expected `=` after the constant name, found `5`", 1, 9);
    }

    #[test]
    fn err_global_without_colon() {
        one_err("global g i64 = 0\n", "expected `:` after the global name, found `i64`", 1, 10);
    }

    #[test]
    fn err_global_bad_type() {
        one_err("global g: i65 = 0\n", "expected a type (`i32`, `i64`, `f32` or `f64`), found `i65`", 1, 11);
    }

    #[test]
    fn import_without_signature() {
        let m = ok("import std.println\nimport a.b.c() -> i64\n");
        let Item::Import(i) = &m.items[0] else { panic!() };
        assert!(!i.has_sig && i.params.is_empty() && i.results.is_empty());
        let Item::Import(i) = &m.items[1] else { panic!() };
        assert!(i.has_sig && i.results.len() == 1);
        one_err("import std.println -> i64\n", "end of line", 1, 20);
    }

    #[test]
    fn err_import_without_namespace() {
        one_err("import println(i64)\n", "must be qualified", 1, 8);
    }

    #[test]
    fn err_import_missing_result_type() {
        one_err("import std.x(i64) ->\n", "expected a type", 1, 21);
    }

    #[test]
    fn err_empty_result_list() {
        one_err("fn f() -> () {\n ret\n}\n", "`-> ()` is not allowed", 1, 11);
    }

    #[test]
    fn err_param_without_colon() {
        one_err("fn f(a i64) {\n ret\n}\n", "expected `:` after the parameter name, found `i64`", 1, 8);
    }

    #[test]
    fn err_bad_top_level_word() {
        one_err("foo bar\n", "expected `db`, `dw`, `dd`, `dq` or `resb` after the data name `foo`, found `bar`", 1, 5);
        one_err("foo\n", "after the data name `foo`, found end of line", 1, 4);
    }

    #[test]
    fn err_junk_at_top_level() {
        one_err("= 3\n", "expected an item", 1, 1);
        one_err("}\n", "unexpected `}`", 1, 1);
    }

    #[test]
    fn err_export_something_else() {
        one_err("export x\n", "expected `fn` or `global` after `export`, found `x`", 1, 8);
    }

    #[test]
    fn err_table_missing_comma() {
        one_err("table [f g]\n", "expected `,` or `]` after the table entry, found `g`", 1, 10);
        one_err("table f\n", "expected `[` after `table`, found `f`", 1, 7);
    }

    #[test]
    fn err_label_list_entry() {
        one_err("fn f() {\n    br_table t, [a, 1], d\n    ret\n}\n", "expected a label name, found `1`", 2, 21);
    }

    #[test]
    fn err_signature_element_not_a_type() {
        one_err("fn f() {\n    call_indirect (i64, x) -> i64, i(a)\n    ret\n}\n", "expected a type", 2, 25);
    }

    #[test]
    fn err_reg_without_names() {
        one_err("fn f() {\n    reg\n    ret\n}\n", "expected a register name, found end of line", 2, 8);
    }

    #[test]
    fn err_statement_starts_with_number() {
        one_err("fn f() {\n    5 6\n    ret\n}\n", "expected an instruction, found `5`", 2, 5);
    }

    #[test]
    fn err_brace_after_label() {
        one_err("fn f() {\nx: }\n", "`}` must stand alone on its line", 2, 4);
    }

    #[test]
    fn err_junk_after_closing_brace() {
        one_err("fn f() {\n ret\n} nop\n", "expected end of line after `}`, found `nop`", 3, 3);
    }

    #[test]
    fn err_open_brace_must_end_line() {
        let d = errs("fn f() { ret\n}\n");
        assert!(d[0].message.contains("expected end of line after `{`, found `ret`"), "{:?}", d);
        assert_eq!((d[0].line, d[0].column), (1, 10));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn err_brace_on_next_line() {
        let d = errs("fn f()\n{\n ret\n}\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("same line"));
        assert_eq!((d[0].line, d[0].column), (2, 1));
    }

    #[test]
    fn err_unclosed_function_is_one_error() {
        let src = "fn f() {\n    nop\n    nop\n    mov a, 1\n    ret\n";
        let e = one_err(src, "expected `}` to close function `f`, found end of file", 6, 1);
        assert_eq!(e.note.as_deref(), Some("the function starts at line 1"));
        // lots of lines, still exactly one error
        let mut long = String::from("fn f() {\n");
        for _ in 0..300 {
            long.push_str("    i64.add a, b, c\n");
        }
        assert_eq!(errs(&long).len(), 1);
    }

    #[test]
    fn err_missing_close_brace_before_next_function() {
        let d = errs("fn a() {\n    nop\nfn b() {\n    ret\n}\nconst X = 1\n");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert!(d[0].message.contains("expected `}` to close function `a`, found `fn`"));
        assert_eq!((d[0].line, d[0].column), (3, 1));
    }

    #[test]
    fn err_too_deeply_nested() {
        let src = format!("fn f() {{\n    mov a, {}x{}\n    ret\n}}\n", "f(".repeat(200), ")".repeat(200));
        let d = errs(&src);
        assert!(d.iter().any(|e| e.message.contains("nested too deeply")), "{:?}", d);
        let src = format!("fn f() {{\n    mov a, {}x{}\n}}\n", "(".repeat(5000), ")".repeat(5000));
        assert!(!errs(&src).is_empty());
    }

    // ---- several errors and recovery ----------------------------------------------------

    #[test]
    fn many_errors_in_one_file() {
        let src = "\
const X 5
global g i64 = 0
import println(i64)
fn f(a i64) {
    mov a, )
    i64.add a b
    ret
}
table [f g]
q
";
        let d = errs(src);
        let lines: Vec<u32> = d.iter().map(|e| e.line).collect();
        // const, global, import, fn header (its body is skipped), table, q
        assert_eq!(lines, vec![1, 2, 3, 4, 9, 10], "{:?}", d);
    }

    #[test]
    fn recovery_inside_a_body_keeps_going() {
        let src = "fn f() {\n    mov a, )\n    nop\n    i64.add a b\n    ret 0 0\n    ret\n}\nconst OK = 1\nbad\n";
        let d = errs(src);
        let lines: Vec<(u32, u32)> = d.iter().map(|e| (e.line, e.column)).collect();
        assert_eq!(lines, vec![(2, 12), (4, 15), (5, 11), (9, 4)], "{:?}", d);
    }

    #[test]
    fn recovery_resumes_with_good_items_after_a_bad_line() {
        let src = "const X 5\nconst Y = 6\nfn f() {\n ret\n}\n";
        let (toks, lex_diags) = lex(src, "t.efa");
        assert!(lex_diags.is_empty());
        let (m, d) = parse(&toks, src, "t.efa");
        assert_eq!(d.len(), 1);
        // the good items after the bad line are in the module
        assert_eq!(m.items.len(), 2);
        assert!(matches!(&m.items[0], Item::Const(c) if c.name.name == "Y"));
        assert!(matches!(&m.items[1], Item::Func(_)));
    }

    #[test]
    fn recovery_keeps_statements_around_a_bad_one() {
        let src = "fn f() {\n    nop\n    mov a, )\n    ret\n}\n";
        let (toks, _) = lex(src, "t.efa");
        let (m, d) = parse(&toks, src, "t.efa");
        assert_eq!(d.len(), 1);
        let f = func(&m, 0);
        let mnemonics: Vec<&str> = f
            .body
            .iter()
            .map(|s| match &s.kind {
                StmtKind::Inst(i) => i.mnemonic.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(mnemonics, ["nop", "ret"]);
        assert_eq!(f.end, Span::new(5, 1, 1));
    }

    #[test]
    fn bad_header_skips_the_whole_function() {
        // one error for the header, none for the body lines
        let d = errs("fn f(a i64) {\n    mov a, )\n    zzz zzz\n}\nconst X = 1\n");
        assert_eq!(d.len(), 1, "{:?}", d);
    }

    #[test]
    fn diagnostics_carry_file_and_source_line() {
        let d = errs("fn f() {\n    i64.add a b\n}\n");
        assert_eq!(d[0].file, "t.efa");
        assert_eq!(d[0].source_line, "    i64.add a b");
        assert!(d[0].to_string().contains("t.efa:2:15"));
    }

    #[test]
    fn lexer_errors_do_not_stop_the_parser() {
        // the stray `@` is the lexer's error; the parser still sees a valid file around it
        let d = errs("const A = 1\nconst B = @ 2\nconst C 3\n");
        let lines: Vec<u32> = d.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![2, 3]);
    }

    #[test]
    fn no_cascade_after_lexer_errors() {
        for src in [
            "z db \"\\xZZ\"\n",
            "x db ''\n",
            "y db 'ab'\n",
            "w db 170141183460469231731687303715884105728 abc\n",
            "fn f() {\n    mov a, 'ab'\n    ret\n}\n",
        ] {
            assert_eq!(errs(src).len(), 1, "{:?}", src);
        }
    }

    #[test]
    fn brace_on_a_statement_line_closes_the_function() {
        let d = errs("fn g() { ret }\n");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert!(d[0].message.contains("end of line after `{`"));
        let d = errs("fn g() {\n    ret }\nconst X = 1\n");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert!(d[0].message.contains("must stand alone"));
        assert_eq!((d[0].line, d[0].column), (2, 9));
        let d = errs("fn g() {\n    ret a, b }\n    nop\n");
        assert_eq!(d.len(), 2, "{:?}", d); // the brace, then the stray `nop` at top level
    }

    // ---- robustness ---------------------------------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                (self.next() >> 11) as usize % n
            }
        }
    }

    #[test]
    fn never_panics_on_mutated_source() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let alphabet: Vec<char> = "ab_9 ,:=-(){}[]\"'\\;\n>.xi64".chars().collect();
        let seeds: [&str; 3] = [
            TOUR,
            "export fn sum(n: i64) -> i64 {\n    reg acc, t\nloop:\n    br_if t, done\n    jmp loop\ndone:\n    ret acc\n}\n",
            "import std.println(i64, i64)\nmsg db \"hi\", 10\nexport fn main() {\n    call std.println(msg, msg.len)\n    ret\n}\n",
        ];
        let mut runs = 0;
        for seed in seeds {
            let base: Vec<char> = seed.chars().collect();
            for _ in 0..140 {
                let mut s = base.clone();
                for _ in 0..1 + rng.below(5) {
                    let at = rng.below(s.len());
                    match rng.below(6) {
                        0 => {
                            if !s.is_empty() {
                                s.remove(at);
                            }
                        }
                        1 => {
                            if !s.is_empty() {
                                let c = s[at];
                                s.insert(at, c);
                            }
                        }
                        2 => {
                            if at + 1 < s.len() {
                                s.swap(at, at + 1);
                            }
                        }
                        3 => {
                            let c = alphabet[rng.below(alphabet.len())];
                            s.insert(at.min(s.len()), c);
                        }
                        4 => {
                            // delete a whole line
                            let text: String = s.iter().collect();
                            let mut lines: Vec<&str> = text.split('\n').collect();
                            let l = rng.below(lines.len());
                            lines.remove(l);
                            s = lines.join("\n").chars().collect();
                        }
                        _ => s.truncate(at),
                    }
                }
                let text: String = s.into_iter().collect();
                let _ = parse_src(&text, "fuzz.efa");
                runs += 1;
            }
        }
        assert!(runs >= 400);
    }

    #[test]
    fn never_panics_on_mutated_tokens() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (base, d) = lex(TOUR, "t.efa");
        assert!(d.is_empty());
        for _ in 0..300 {
            let mut t = base.clone();
            for _ in 0..1 + rng.below(6) {
                if t.len() < 2 {
                    break;
                }
                let at = rng.below(t.len() - 1);
                match rng.below(4) {
                    0 => {
                        t.remove(at);
                    }
                    1 => {
                        let x = t[at].clone();
                        t.insert(at, x);
                    }
                    2 => {
                        let other = rng.below(t.len() - 1);
                        t.swap(at, other);
                    }
                    _ => {
                        // replace by a random other token
                        let other = rng.below(t.len());
                        t[at] = t[other].clone();
                    }
                }
            }
            // sometimes drop the trailing Eof/Newline too
            if rng.below(4) == 0 {
                let cut = rng.below(t.len() + 1);
                t.truncate(cut);
            }
            let _ = parse(&t, TOUR, "t.efa");
        }
        // degenerate streams
        let _ = parse(&[], "", "t.efa");
        let (only_eof, _) = lex("", "t.efa");
        let _ = parse(&only_eof, "", "t.efa");
        // a source shorter than the token spans claim
        let _ = parse(&base, "", "t.efa");
    }

    #[test]
    fn never_panics_on_hostile_shapes() {
        for src in [
            "fn",
            "fn f(",
            "fn f() -> (",
            "fn f() {",
            "fn f() {\nx:",
            "{",
            "}}}}",
            "(((((",
            "import",
            "import a.b(",
            "table [",
            "table [a,",
            "x db",
            "x db ,",
            "export",
            "export export",
            "global mut",
            "fn f() {\n call (((\n}",
            "fn f() {\n [[[\n}",
            "fn f() {\n a: b: c:\n}",
            "fn f() {\n ret (a, b) -> (\n}",
        ] {
            let _ = parse_src(src, "t.efa");
            let _ = parse_src(&src.replace('\n', "\r\n"), "t.efa");
        }
    }
}
