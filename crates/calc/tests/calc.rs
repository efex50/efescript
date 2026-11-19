use calc::*;

fn ev(s: &str) -> Result<Value, CalcError> {
    eval(&parse(s).unwrap(), &NoEnv)
}
fn int(s: &str) -> i128 {
    match ev(s).unwrap() {
        Value::Int(n) => n,
        v => panic!("{s} -> {v:?}"),
    }
}
fn kind(s: &str) -> CalcErrorKind {
    ev(s).unwrap_err().kind
}
fn perr(s: &str) -> CalcError {
    parse(s).unwrap_err()
}

#[test]
fn precedence_table() {
    // (source, expected) pairs where precedence decides the answer
    let cases: &[(&str, i128)] = &[
        ("1 + 2 * 3", 7),
        ("2 * 3 + 1", 7),
        ("10 - 2 * 3", 4),
        ("1 << 2 + 1", 8),      // + above <<
        ("1 | 2 & 3", 3),       // & above |
        ("6 ^ 3 & 1", 7),       // & above ^
        ("1 | 6 ^ 3", 5),       // ^ above |
        ("8 >> 1 & 3", 0),      // >> above &
        ("7 % 4 * 2", 6),
        ("-2 * 3", -6),
        ("~0 & 0xff", 255),
        ("2 + 3 << 1", 10),
    ];
    for (s, v) in cases {
        assert_eq!(int(s), *v, "{s}");
    }
    assert_eq!(ev("1 + 2 < 4 && 3 > 2"), Ok(Value::Bool(true)));
    assert_eq!(ev("1 | 2 == 3"), Ok(Value::Bool(true))); // | above ==
    assert_eq!(ev("true || false && false"), Ok(Value::Bool(true))); // && above ||
    assert_eq!(ev("1 < 2 == true"), Ok(Value::Bool(true))); // left assoc comparisons
}

#[test]
fn left_associativity() {
    assert_eq!(int("10 - 3 - 2"), 5);
    assert_eq!(int("100 / 5 / 2"), 10);
    assert_eq!(int("100 % 30 % 7"), 3);
    assert_eq!(int("1 << 2 << 3"), 32);
    assert_eq!(int("256 >> 2 >> 2"), 16);
    let e = parse("a - b - c").unwrap();
    assert_eq!(e.to_string(), "a - b - c");
    match e.kind {
        ExprKind::Binary(BinOp::Sub, l, _) => assert!(matches!(l.kind, ExprKind::Binary(BinOp::Sub, ..))),
        k => panic!("{k:?}"),
    }
}

#[test]
fn unary_ops() {
    assert_eq!(int("-5"), -5);
    assert_eq!(int("- -5"), 5);
    assert_eq!(int("~5"), -6);
    assert_eq!(int("-~5"), 6);
    assert_eq!(ev("!true"), Ok(Value::Bool(false)));
    assert_eq!(ev("!!true"), Ok(Value::Bool(true)));
    assert_eq!(int("2 - -3"), 5);
    assert_eq!(ev("-2.5"), Ok(Value::Float(-2.5)));
    assert!(matches!(kind("!1"), CalcErrorKind::TypeMismatch(_)));
    assert!(matches!(kind("~1.5"), CalcErrorKind::TypeMismatch(_)));
    assert!(matches!(kind("-true"), CalcErrorKind::TypeMismatch(_)));
}

#[test]
fn parens() {
    assert_eq!(int("(1 + 2) * 3"), 9);
    assert_eq!(int("((((7))))"), 7);
    assert_eq!(int("-(1 + 2)"), -3);
    assert!(matches!(parse("(1)").unwrap().kind, ExprKind::Paren(_)));
}

#[test]
fn error_spans() {
    let e = perr("1 + * 2");
    assert_eq!(e.kind, CalcErrorKind::UnexpectedToken("*".into()));
    assert_eq!(e.span, Span::new(4, 5));
    assert!(e.render("1 + * 2").ends_with("    ^"));

    let e = perr("1 +");
    assert_eq!(e.kind, CalcErrorKind::UnexpectedEnd);
    assert_eq!(e.span, Span::new(3, 3));

    let e = perr("");
    assert_eq!(e.kind, CalcErrorKind::UnexpectedEnd);

    let e = perr("1 + (2 * 3");
    assert_eq!(e.kind, CalcErrorKind::UnclosedBracket);
    assert_eq!(e.span, Span::new(4, 5));

    let e = perr("f(1, 2");
    assert_eq!(e.kind, CalcErrorKind::UnclosedBracket);
    assert_eq!(e.span, Span::new(1, 2));

    let e = perr("1 2");
    assert_eq!(e.kind, CalcErrorKind::UnexpectedToken("2".into()));
    assert_eq!(e.span, Span::new(2, 3));

    let e = perr("(1))");
    assert_eq!(e.span, Span::new(3, 4));

    let e = perr("1 $ 2");
    assert_eq!(e.kind, CalcErrorKind::UnexpectedToken("$".into()));
    assert_eq!(e.span, Span::new(2, 3));

    let e = perr("a.");
    assert_eq!(e.kind, CalcErrorKind::UnexpectedEnd);

    // runtime error spans point at the offending node
    let e = ev("1 + 4 / 0").unwrap_err();
    assert_eq!(e.span, Span::new(4, 9));
    let msg = e.to_string();
    assert!(msg.contains("division by zero") && msg.contains("4..9"), "{msg}");
    let _: &dyn std::error::Error = &e;

    let e = parse("1 + x").unwrap();
    let e = eval(&e, &NoEnv).unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::UnknownVariable("x".into()));
    assert_eq!(e.span, Span::new(4, 5));
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let s = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert_eq!(perr(&s).kind, CalcErrorKind::TooDeep);
}

#[test]
fn overflow_checks() {
    let max = i128::MAX.to_string();
    assert_eq!(kind(&format!("{max} + 1")), CalcErrorKind::Overflow);
    assert_eq!(kind(&format!("-{max} - 2")), CalcErrorKind::Overflow);
    assert_eq!(kind(&format!("{max} * 2")), CalcErrorKind::Overflow);
    assert_eq!(int(&format!("{max} - 1")), i128::MAX - 1);
    // i128::MIN / -1 and % -1
    assert_eq!(kind(&format!("(-{max} - 1) / -1")), CalcErrorKind::Overflow);
    assert_eq!(kind(&format!("(-{max} - 1) % -1")), CalcErrorKind::Overflow);
    assert_eq!(kind(&format!("-(-{max} - 1)")), CalcErrorKind::Overflow);
    assert!(matches!(perr_num("170141183460469231731687303715884105728"), CalcErrorKind::InvalidNumber(_)));
}

fn perr_num(s: &str) -> CalcErrorKind {
    lex(s).unwrap_err().kind
}

#[test]
fn division_and_remainder_by_zero() {
    assert_eq!(kind("1 / 0"), CalcErrorKind::DivisionByZero);
    assert_eq!(kind("1 % 0"), CalcErrorKind::DivisionByZero);
    assert_eq!(kind("1.0 / 0"), CalcErrorKind::DivisionByZero);
    assert_eq!(kind("1 / 0.0"), CalcErrorKind::DivisionByZero);
    assert_eq!(kind("1 / (2 - 2)"), CalcErrorKind::DivisionByZero);
    assert_eq!(int("7 / 2"), 3);
    assert_eq!(int("-7 / 2"), -3);
    assert_eq!(int("-7 % 3"), -1);
}

#[test]
fn shift_range() {
    assert_eq!(int("1 << 0"), 1);
    assert_eq!(int("1 << 100"), 1i128 << 100);
    assert_eq!(int("-1 >> 127"), -1);
    assert_eq!(kind("1 << 128"), CalcErrorKind::Overflow);
    assert_eq!(kind("1 >> 128"), CalcErrorKind::Overflow);
    assert_eq!(kind("1 << -1"), CalcErrorKind::Overflow);
    assert_eq!(kind("1 >> 1000"), CalcErrorKind::Overflow);
}

#[test]
fn type_mismatches() {
    for s in ["1 + true", "true * 2", "1.5 % 2", "2 % 1.5", "1.5 << 1", "1 & 1.0", "1.0 | 1", "1 && 2", "true && 1",
              "1 || true", "true < false", "1 == true", "true + true", "1.5 ^ 2"] {
        assert!(matches!(kind(s), CalcErrorKind::TypeMismatch(_)), "{s}");
    }
    assert_eq!(ev("true == true"), Ok(Value::Bool(true)));
    assert_eq!(ev("true != false"), Ok(Value::Bool(true)));
}

#[test]
fn mixed_int_float() {
    assert_eq!(ev("1 + 2.5"), Ok(Value::Float(3.5)));
    assert_eq!(ev("2.5 * 2"), Ok(Value::Float(5.0)));
    assert_eq!(ev("7 / 2.0"), Ok(Value::Float(3.5)));
    assert_eq!(ev("7.0 / 2"), Ok(Value::Float(3.5)));
    assert_eq!(ev("1 - 0.5"), Ok(Value::Float(0.5)));
    assert_eq!(ev("1 < 1.5"), Ok(Value::Bool(true)));
    assert_eq!(ev("2 == 2.0"), Ok(Value::Bool(true)));
    assert_eq!(ev("1e3"), Ok(Value::Float(1000.0)));
    assert_eq!(ev("1.5e-1 + 0"), Ok(Value::Float(0.15)));
    assert_eq!(ev("2E+2"), Ok(Value::Float(200.0)));
}

#[test]
fn bool_ops() {
    assert_eq!(ev("true && false"), Ok(Value::Bool(false)));
    assert_eq!(ev("false || true"), Ok(Value::Bool(true)));
    assert_eq!(ev("1 < 2 && 2 < 3"), Ok(Value::Bool(true)));
    assert_eq!(ev("1 >= 2 || 3 <= 3"), Ok(Value::Bool(true)));
    assert_eq!(ev("1 != 2"), Ok(Value::Bool(true)));
    // short circuit: rhs division by zero is never evaluated
    assert_eq!(ev("false && 1 / 0 == 1"), Ok(Value::Bool(false)));
    assert_eq!(ev("true || 1 / 0 == 1"), Ok(Value::Bool(true)));
    assert_eq!(kind("true && 1 / 0 == 1"), CalcErrorKind::DivisionByZero);
}

struct TestEnv;
impl Env for TestEnv {
    fn var(&self, name: &str) -> Option<Value> {
        match name {
            "x" => Some(Value::Int(10)),
            "Sprite.size" => Some(Value::Int(16)),
            "A.B.c" => Some(Value::Float(0.5)),
            _ => None,
        }
    }
    fn call(&self, name: &str, args: &[Value]) -> Option<Result<Value, CalcError>> {
        match (name, args) {
            ("max", [Value::Int(a), Value::Int(b)]) => Some(Ok(Value::Int(*a.max(b)))),
            ("fail", _) => Some(Err(CalcError::new(CalcErrorKind::Overflow, Span::default()))),
            ("max", _) => Some(Err(CalcError::new(CalcErrorKind::TypeMismatch("max".into()), Span::default()))),
            _ => None,
        }
    }
}

#[test]
fn fields_and_calls_via_env() {
    let r = |s: &str| eval(&parse(s).unwrap(), &TestEnv);
    assert_eq!(r("Sprite.size * 2"), Ok(Value::Int(32)));
    assert_eq!(r("A.B.c + 1"), Ok(Value::Float(1.5)));
    assert_eq!(r("max(x, 3) + 1"), Ok(Value::Int(11)));
    assert_eq!(r("max(1, max(2, 3))"), Ok(Value::Int(3)));
    assert_eq!(r("max(x, 3).y").unwrap_err().kind, CalcErrorKind::TypeMismatch("field access on a non-name expression".into()));
    let e = r("Sprite.nope").unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::UnknownVariable("Sprite.nope".into()));
    assert_eq!(e.span, Span::new(0, 11));
    let e = r("1 + nofn(2)").unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::UnknownFunction("nofn".into()));
    assert_eq!(e.span, Span::new(4, 11));
    // error from Env::call gets the call node's span
    let e = r("2 * fail(1)").unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::Overflow);
    assert_eq!(e.span, Span::new(4, 11));
    // parse shapes
    assert!(matches!(parse("f()").unwrap().kind, ExprKind::Call(ref n, ref a) if n == "f" && a.is_empty()));
    assert!(matches!(parse("a.b.c").unwrap().kind, ExprKind::Field(..)));
    assert_eq!(parse("a.b.c").unwrap().dotted_path().as_deref(), Some("a.b.c"));
    // postfix binds tighter than unary
    assert!(matches!(parse("-a.b").unwrap().kind, ExprKind::Unary(UnOp::Neg, _)));
}

#[test]
fn vars_env() {
    let mut v = Vars::new();
    v.set("x", 4i128);
    v.set("y", 1.5);
    v.set("flag", true);
    assert_eq!(eval(&parse("x * 2 + y").unwrap(), &v), Ok(Value::Float(9.5)));
    assert_eq!(eval(&parse("flag && x > 3").unwrap(), &v), Ok(Value::Bool(true)));
    assert_eq!(v.unset("x"), Some(Value::Int(4)));
    assert!(eval(&parse("x").unwrap(), &v).is_err());
    assert_eq!(eval(&parse("f(1)").unwrap(), &v).unwrap_err().kind, CalcErrorKind::UnknownFunction("f".into()));
}

fn simp(s: &str) -> String {
    let mut v = Vars::new();
    v.set("k", 3i128);
    simplify(&parse(s).unwrap(), &v).unwrap().to_string()
}

#[test]
fn simplify_with_unknowns() {
    assert_eq!(simp("1 + 2 * 3"), "7");
    assert_eq!(simp("x + 1 + 2"), "x + 1 + 2"); // left assoc: no reassociation
    assert_eq!(simp("x + (1 + 2)"), "x + 3");
    assert_eq!(simp("k * 2 + x"), "6 + x");
    assert_eq!(simp("x + 0"), "x");
    assert_eq!(simp("0 + x"), "x");
    assert_eq!(simp("x - 0"), "x");
    assert_eq!(simp("x * 1"), "x");
    assert_eq!(simp("1 * x"), "x");
    assert_eq!(simp("x / 1"), "x");
    assert_eq!(simp("x * 0"), "0");
    assert_eq!(simp("0 * x"), "0");
    assert_eq!(simp("x * (k - 3)"), "0");
    assert_eq!(simp("0 - x"), "0 - x");
    assert_eq!(simp("(k + 1) * x"), "4 * x");
    assert_eq!(simp("(x + k)"), "(x + 3)");
    assert_eq!(simp("(1 + 2)"), "3");
    assert_eq!(simp("-(1 + 2) + x"), "-3 + x");
    assert_eq!(simp("false && x"), "false");
    assert_eq!(simp("true || x"), "true");
    assert_eq!(simp("x && false"), "x && false");
    assert_eq!(simp("f(1 + 1, x)"), "f(2, x)");
    assert_eq!(simp("a.b + k"), "a.b + 3");
    assert_eq!(simp("2 > 1"), "true");
    assert_eq!(simp("1.5 + 1"), "2.5");
}

#[test]
fn simplify_propagates_errors_with_spans() {
    let e = simplify(&parse("x + 4 / 0").unwrap(), &NoEnv).unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::DivisionByZero);
    assert_eq!(e.span, Span::new(4, 9));
    let e = simplify(&parse("x + (1 + true)").unwrap(), &NoEnv).unwrap_err();
    assert!(matches!(e.kind, CalcErrorKind::TypeMismatch(_)));
    assert_eq!(e.span, Span::new(5, 13));
    let e = simplify(&parse("1 << 200").unwrap(), &NoEnv).unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::Overflow);
}

#[test]
fn simplify_calls_env_when_constant() {
    let e = simplify(&parse("max(2, 1 + 4) + y").unwrap(), &TestEnv).unwrap();
    assert_eq!(e.to_string(), "5 + y");
    let e = simplify(&parse("Sprite.size + 1").unwrap(), &TestEnv).unwrap();
    assert_eq!(e.to_string(), "17");
}

#[test]
fn narrowing_ranges() {
    use IntWidth::*;
    let table = [
        (I8, -128, 127),
        (I16, -32768, 32767),
        (I32, i32::MIN as i128, i32::MAX as i128),
        (I64, i64::MIN as i128, i64::MAX as i128),
        (U8, 0, 255),
        (U16, 0, 65535),
        (U32, 0, u32::MAX as i128),
        (U64, 0, u64::MAX as i128),
    ];
    for (w, lo, hi) in table {
        assert_eq!(w.range(), (lo, hi));
        assert_eq!(Value::Int(lo).to_int_of(w), Ok(lo));
        assert_eq!(Value::Int(hi).to_int_of(w), Ok(hi));
        assert_eq!(Value::Int(lo - 1).to_int_of(w).unwrap_err().kind, CalcErrorKind::Overflow);
        assert_eq!(Value::Int(hi + 1).to_int_of(w).unwrap_err().kind, CalcErrorKind::Overflow);
    }
    assert!(matches!(Value::Float(1.0).to_int_of(U8).unwrap_err().kind, CalcErrorKind::TypeMismatch(_)));
    assert!(matches!(Value::Bool(true).to_int_of(U8).unwrap_err().kind, CalcErrorKind::TypeMismatch(_)));
    assert_eq!(IntWidth::from_name("u16"), Some(U16));
    assert_eq!(IntWidth::from_name("x"), None);
    assert_eq!(Value::Int(3).to_f64(), Ok(3.0));
    assert_eq!(Value::Float(0.5).to_f32(), Ok(0.5f32));
    assert_eq!(Value::Float(1e300).to_f32().unwrap_err().kind, CalcErrorKind::Overflow);
    assert!(Value::Bool(true).to_f64().is_err());
    let e = Value::Int(300).to_int_of(U8).unwrap_err().with_span(Span::new(1, 2));
    assert_eq!(e.span, Span::new(1, 2));
}

#[test]
fn radix_and_number_literals() {
    assert_eq!(int("0xff"), 255);
    assert_eq!(int("0XFF"), 255);
    assert_eq!(int("0b1010"), 10);
    assert_eq!(int("0o17"), 15);
    assert_eq!(int("1_000"), 1000);
    assert_eq!(int("0xdead_beef"), 0xdead_beef);
    assert_eq!(int("0"), 0);
    assert_eq!(int("007"), 7);
    assert_eq!(int("0xff + 0b1"), 256);
    for bad in ["0x", "0b2", "0o8", "0xg", "12abc", "0b12", "1e5x"] {
        let e = lex(bad).unwrap_err();
        assert!(matches!(e.kind, CalcErrorKind::InvalidNumber(_)), "{bad}");
        assert_eq!(e.span, Span::new(0, bad.len()), "{bad}");
    }
    assert_eq!(lex("1e999").unwrap_err().kind, CalcErrorKind::InvalidNumber("1e999".into()));
    // `1.` followed by non-digit: Int then Dot
    let t = lex("1.x").unwrap();
    assert_eq!(t.iter().map(|t| t.kind.clone()).collect::<Vec<_>>(),
        vec![TokenKind::Int(1), TokenKind::Dot, TokenKind::Ident("x".into())]);
    assert_eq!(parse_value("-0x10"), Ok(Value::Int(-16)));
    assert_eq!(parse_value("2.5"), Ok(Value::Float(2.5)));
    assert_eq!(parse_value("true"), Ok(Value::Bool(true)));
}

#[test]
fn lexer_tokens_and_spans() {
    use TokenKind::*;
    let t = lex("ab+ 12 << >= ==!=&&||&|^~!<>%/*-,.()").unwrap();
    let kinds: Vec<_> = t.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            Ident("ab".into()), Plus, Int(12), Shl, Ge, EqEq, NotEq, AndAnd, OrOr, Amp, Pipe, Caret, Tilde,
            Bang, Lt, Gt, Percent, Slash, Star, Minus, Comma, Dot, LParen, RParen
        ]
    );
    assert_eq!(t[0].span, Span::new(0, 2));
    assert_eq!(t[1].span, Span::new(2, 3));
    assert_eq!(t[2].span, Span::new(4, 6));
    assert_eq!(t[3].span, Span::new(7, 9));
    assert_eq!(lex("  ").unwrap(), vec![]);
    // multi-byte identifiers: spans are byte offsets
    let t = lex("é + x").unwrap();
    assert_eq!(t[0].span, Span::new(0, 2));
    assert_eq!(t[2].span, Span::new(5, 6));
    // a lone `=` is not a token
    assert_eq!(lex("a = 1").unwrap_err().span, Span::new(2, 3));
}

#[test]
fn external_token_source() {
    // tokens from a foreign lexer, with their own spans
    let toks = vec![
        Token::new(TokenKind::Int(2), 10, 11),
        Token::new(TokenKind::Star, 12, 13),
        Token::new(TokenKind::Ident("x".into()), 14, 15),
    ];
    let e = parse_tokens(&toks).unwrap();
    assert_eq!(e.span, Span::new(10, 15));
    assert_eq!(e.to_string(), "2 * x");
    let e = parse_tokens(&toks[..2]).unwrap_err();
    assert_eq!(e.kind, CalcErrorKind::UnexpectedEnd);
    assert_eq!(e.span, Span::new(13, 13));

    // parse one expression and stop at a comma (assembler operand style)
    let mut lx = Lexer::new("1 + 2, 3");
    let mut p = Parser::new(&mut lx);
    let a = p.parse_expr().unwrap();
    assert_eq!(a.to_string(), "1 + 2");
    assert_eq!(p.peek().unwrap().unwrap().kind, TokenKind::Comma);
    assert!(p.finish().is_err());

    // custom TokenSource
    struct Two(u8);
    impl TokenSource for Two {
        fn next_token(&mut self) -> Result<Option<Token>, CalcError> {
            self.0 += 1;
            Ok(match self.0 {
                1 => Some(Token::new(TokenKind::Int(1), 0, 1)),
                2 => Some(Token::new(TokenKind::Plus, 1, 2)),
                3 => Some(Token::new(TokenKind::Int(1), 2, 3)),
                _ => None,
            })
        }
    }
    let mut s = Two(0);
    let mut p = Parser::new(&mut s);
    let e = p.parse_expr().unwrap();
    p.finish().unwrap();
    assert_eq!(eval(&e, &NoEnv), Ok(Value::Int(2)));
}

#[test]
fn round_trip_printing() {
    let srcs = [
        "1 + 2 * 3", "(1 + 2) * 3", "a - (b - c)", "a - b - c", "-(a + b)", "- -a", "~a & 0xff", "!(a && b) || c",
        "a << 2 + 1", "f(1, g(2), x.y) + z.w.v", "(a + b).c", "a == b != c", "1.5 * 2e10 - 3.25e-7",
        "a | b ^ c & d", "a % b / c * d", "true && !false", "-a.b", "((a))", "x >= 1 && y <= 2 || z > 3 && w < 4",
        "1 - -2", "f()", "a.b.c.d",
    ];
    for s in srcs {
        let e = parse(s).unwrap();
        let printed = e.to_string();
        let e2 = parse(&printed).unwrap_or_else(|err| panic!("{s} -> {printed}: {err}"));
        assert_eq!(e, e2, "{s} -> {printed}");
        assert_eq!(printed, e2.to_string());
    }
    assert_eq!(parse("1+2*3").unwrap().to_string(), "1 + 2 * 3");
}

#[test]
fn printer_parenthesizes_hand_built_trees() {
    let sp = Span::default();
    let n = |v: i128| Box::new(Expr::new(ExprKind::Int(v), sp));
    let add = Box::new(Expr::new(ExprKind::Binary(BinOp::Add, n(1), n(2)), sp));
    let e = Expr::new(ExprKind::Binary(BinOp::Mul, add.clone(), n(3)), sp);
    assert_eq!(e.to_string(), "(1 + 2) * 3");
    let e = Expr::new(ExprKind::Binary(BinOp::Sub, n(3), add), sp);
    assert_eq!(e.to_string(), "3 - (1 + 2)");
    let e = Expr::new(ExprKind::Unary(UnOp::Neg, Box::new(Expr::new(ExprKind::Binary(BinOp::Add, n(1), n(2)), sp))), sp);
    assert_eq!(e.to_string(), "-(1 + 2)");
    let neg = Box::new(Expr::new(ExprKind::Int(-5), sp));
    let e = Expr::new(ExprKind::Field(neg, "x".into()), sp);
    assert_eq!(e.to_string(), "(-5).x");
}

#[test]
fn expr_spans() {
    let e = parse("  a + b * c ").unwrap();
    assert_eq!(e.span, Span::new(2, 11));
    let ExprKind::Binary(_, l, r) = e.kind else { panic!() };
    assert_eq!(l.span, Span::new(2, 3));
    assert_eq!(r.span, Span::new(6, 11));
    let e = parse("-(a)").unwrap();
    assert_eq!(e.span, Span::new(0, 4));
    let e = parse("f(1, 2).x").unwrap();
    assert_eq!(e.span, Span::new(0, 9));
}
