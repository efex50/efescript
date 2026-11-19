use calc::{eval, parse, parse_value, simplify, Value, Vars};
use clap::{Parser, ValueEnum};
use std::io::{self, IsTerminal, Read};

#[derive(Parser, Debug)]
#[command(version, about = "Evaluate, simplify or print arithmetic expressions", long_about = None)]
struct Args {
    /// Expression (otherwise read from piped stdin)
    #[arg(short, long)]
    string: Option<String>,

    /// Variable `name=value` (repeatable; value may be int, float, bool or a constant expression)
    #[arg(short, long)]
    var: Vec<String>,

    /// Fold constants and print the (possibly partial) result as infix
    #[arg(long, default_value_t = false)]
    simplify: bool,

    /// Print the parsed expression instead of evaluating it (repeatable)
    #[arg(long, value_enum)]
    print: Vec<PrintType>,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
enum PrintType {
    Infix,
    Debug,
}

fn main() {
    let args = Args::parse();
    let src = match args.string {
        Some(s) => s,
        None if !io::stdin().is_terminal() => {
            let mut buf = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut buf) {
                eprintln!("error: cannot read stdin: {e}");
                std::process::exit(1);
            }
            buf.trim().to_string()
        }
        None => {
            eprintln!("error: no expression given; use -s \"expr\" or pipe one in");
            std::process::exit(2);
        }
    };

    let mut vars = Vars::new();
    for v in &args.var {
        let Some((key, val)) = v.split_once('=') else {
            eprintln!("error: invalid variable `{v}`, expected name=value");
            std::process::exit(2);
        };
        match parse_value(val) {
            Ok(value) => {
                vars.set(key.trim(), value);
            }
            Err(e) => {
                eprintln!("variable `{key}`: {}", e.render(val));
                std::process::exit(2);
            }
        }
    }

    let fail = |e: calc::CalcError| -> ! {
        eprintln!("{}", e.render(&src));
        std::process::exit(1);
    };

    let mut expr = parse(&src).unwrap_or_else(|e| fail(e));
    if args.simplify {
        expr = simplify(&expr, &vars).unwrap_or_else(|e| fail(e));
    }

    if !args.print.is_empty() {
        for p in &args.print {
            match p {
                PrintType::Infix => println!("{expr}"),
                PrintType::Debug => println!("{expr:#?}"),
            }
        }
    } else if args.simplify {
        println!("{expr}");
    } else {
        let v: Value = eval(&expr, &vars).unwrap_or_else(|e| fail(e));
        println!("{v}");
    }
}
