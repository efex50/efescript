# calc

Expression front end: a char-based lexer, a Pratt parser generic over a token source, an evaluator with checked integer arithmetic, and a constant folder. Meant for a future language and for constant expressions in the `.efa` assembler.

```rust
use calc::{parse, eval, simplify, Vars, Value};

let e = parse("(x + 2) * 3 << 1").unwrap();
let mut vars = Vars::new();
vars.set("x", 4i128);
assert_eq!(eval(&e, &vars).unwrap(), Value::Int(36));
println!("{}", simplify(&parse("y * 1 + 2 * 3").unwrap(), &vars).unwrap()); // y + 6
```

- Syntax: ints (decimal, `0x`, `0b`, `0o`, `_` separators), floats, `true`/`false`, names, `a.b` fields, `f(x, y)` calls, unary `- ! ~`, binary `* / %`, `+ -`, `<< >>`, `&`, `^`, `|`, comparisons, `&&`, `||` (C/Rust precedence, all left associative).
- Errors are `CalcError { kind, span }` with byte-offset spans; `CalcError::render(src)` draws a caret line.
- Evaluation: `i128` checked arithmetic (`Overflow`, `DivisionByZero`), `Value::{Int, Float, Bool}`, int/float mixing promotes to float.
- `Env` supplies variables (`a.b.c` is looked up as the name `"a.b.c"`) and functions.
- `Value::to_int_of(IntWidth)` range-checks for fixed-width VM integers.
- Plug in your own lexer by implementing `TokenSource` (or pass `&[Token]` to `parse_tokens`).

CLI: `cargo run -p calc --bin cli -- -s "x * 2 + 1" -v x=4`, `--simplify`, `--print infix|debug`.
