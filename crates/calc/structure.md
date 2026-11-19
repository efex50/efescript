# calc

`calc` is an expression front end: a char-based lexer producing spanned tokens, a Pratt parser that accepts tokens from any `TokenSource`, an infix printer, an evaluator with checked `i128` arithmetic, a constant folder, and fixed-width narrowing helpers for a VM. It is meant for a future language and for constant expressions in the `.efa` assembler. It ships a `cli` binary. No workspace crate depends on it yet.

## At a glance
| | |
|---|---|
| Path | `crates/calc` |
| Kind | library (+ binary `cli` behind the `cli` feature) |
| Depends on (workspace) | none |
| Used by | nothing yet |
| Key external deps | `clap` 4.5 (optional, CLI only) |
| Features | `default = ["cli"]`; `cli = ["dep:clap"]` |

## Layout
```text
crates/calc/
├── Cargo.toml
├── README.md
├── tests/calc.rs       # integration tests (24)
└── src/
    ├── lib.rs          # re-exports + parse_value()
    ├── error.rs        # Span, CalcError, CalcErrorKind (Display + std::error::Error, render(src))
    ├── lexer.rs        # Token, TokenKind, TokenSource, TokenIter, Lexer, lex()
    ├── ast.rs          # Expr, ExprKind, UnOp, BinOp (+ precedence), Display (infix printer)
    ├── parser.rs       # Parser (Pratt), parse(), parse_tokens()
    ├── eval.rs         # Value, IntWidth, Env, NoEnv, Vars, eval(), simplify()
    └── bin/cli.rs      # cli binary
```

## Public API
Errors
- `Span { start, end }` (byte offsets; `Span::new`, `to`), `CalcError { kind, span }` (`new`, `with_span`, `render(&str) -> String`).
- `CalcErrorKind`: `UnexpectedToken(String)`, `UnexpectedEnd`, `UnclosedBracket` (span = the opening bracket), `InvalidNumber(String)`, `UnknownVariable(String)`, `UnknownFunction(String)`, `DivisionByZero`, `Overflow`, `TypeMismatch(String)`, `TooDeep` (nesting over 200).

Lexing
- `lex(&str) -> Result<Vec<Token>, CalcError>`; `Token { kind: TokenKind, span }`; `Token::new(kind, start, end)`.
- `TokenKind`: `Int(i128) Float(f64) Ident(String) Plus Minus Star Slash Percent Shl Shr Amp Pipe Caret Tilde Bang EqEq NotEq Lt Le Gt Ge AndAnd OrOr LParen RParen Comma Dot`.
- `trait TokenSource { fn next_token(&mut self) -> Result<Option<Token>, CalcError>; }`, implemented by `Lexer<'a>` and `TokenIter<I: Iterator<Item = Token>>`.
- Literals: `0x`/`0b`/`0o` (either case), `_` separators, decimal, `1.5`, `1e3`/`1.5E-2`; `12abc` and out-of-range numbers are `InvalidNumber` covering the whole literal. `.` is its own token (`1.x` is Int, Dot, Ident). A negative literal is unary minus applied to a positive one.

Parsing
- `parse(&str) -> Result<Expr, CalcError>` (rejects trailing tokens), `parse_tokens(&[Token])`.
- `Parser::new(&mut dyn TokenSource)` with `parse_expr()`, `peek()`, `finish()`: parse one expression and stop at the first token that cannot continue it (for example a `,` between assembler operands).
- Precedence, tightest first: postfix (`.field`, `name(args)`), unary `- ! ~`, `* / %`, `+ -`, `<< >>`, `&`, `^`, `|`, comparisons, `&&`, `||`. All binary operators are left associative (`a < b < c` parses as `(a < b) < c` and then fails type checking). Calls are only allowed on a plain name.
- `true`/`false` parse as `ExprKind::Bool`.

AST
- `Expr { kind: ExprKind, span }`; `PartialEq` compares structure and ignores spans. `ExprKind`: `Int Float Bool Var Unary Binary Field Call Paren`. `Expr::dotted_path()` gives `"a.b.c"` for a name followed by fields.
- `impl Display for Expr` is the infix printer; `parse(&e.to_string()) == e` holds. Parentheses already in the tree (`Paren`) are kept; a hand-built tree gets the parentheses its precedence needs. Simplified trees may contain negative literals, which print as `-5` and reparse as `Unary(Neg, Int(5))`.

Evaluation
- `Value::{Int(i128), Float(f64), Bool(bool)}` (`From` for i128/i64/i32/u64/f64/bool, `Display`, `to_expr`, `type_name`).
- `trait Env { fn var(&self, &str) -> Option<Value>; fn call(&self, &str, &[Value]) -> Option<Result<Value, CalcError>>; }`. `a.b.c` is looked up as one name via `var("a.b.c")`. Errors returned from `call` get the call node's span. `NoEnv`, and `Vars` (HashMap: `new`, `set`, `unset`, pub `map`).
- `eval(&Expr, &dyn Env) -> Result<Value, CalcError>`: checked `+ - * / %` and negation (`Overflow`), `/` and `%` by zero (`DivisionByZero`, also for floats), shift amount outside 0..128 (`Overflow`), int/float promotion to float, `% << >> & | ^ ~` ints only, comparisons give `Bool` (`==`/`!=` also on two bools), `&& || !` need bools (`&&`/`||` short-circuit), anything else `TypeMismatch`.
- `simplify(&Expr, &dyn Env) -> Result<Expr, CalcError>`: folds constant subtrees, substitutes known variables, calls `Env::call` when all arguments are constant (so functions must be pure), keeps unknown names. Identities: `x+0`, `0+x`, `x-0`, `x*1`, `1*x`, `x/1`, `x*0`, `0*x` (with an integer literal), `false && _`, `true || _`. There is no reassociation: `x + 1 + 2` stays as is, `x + (1 + 2)` becomes `x + 3`.
- `IntWidth::{I8,I16,I32,I64,U8,U16,U32,U64}` (`bits`, `signed`, `range`, `from_name`); `Value::to_int_of(width) -> Result<i128, CalcError>` (`Overflow` out of range, `TypeMismatch` for float/bool; spans are `Span::default()`, add one with `with_span`); `Value::to_f32` (`Overflow` if a finite value becomes infinite) and `to_f64`.
- `parse_value(&str) -> Result<Value, CalcError>`: parse and evaluate with no variables.

## CLI (`src/bin/cli.rs`)
`-s/--string EXPR` (or piped stdin), `-v/--var name=value` (repeatable, value parsed with `parse_value`), `--simplify` (print the folded expression), `--print infix|debug` (print the parsed expression, repeatable). Default: evaluate and print the value. Errors print a caret diagram to stderr and exit 1.

## Dropped from the old API
`Tree`, `TreeType`, `TreeTokens`, `Number`, `Symbols`, `FixType`, prefix/postfix parsing and printing, notation auto-detection, `util::split_str`, `util::as_i128`, and the placeholder `add()`. Prefix/postfix were removed because the new grammar (unary operators, comparisons, calls) has no unambiguous prefix or postfix form worth maintaining.

## Tests
`cargo test -p calc`: 24 integration tests in `tests/calc.rs` plus one doc test.
