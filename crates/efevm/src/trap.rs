//! Run-time traps and fiber exits.

use std::fmt;

/// A run-time error that terminates the fiber.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap {
    OutOfBounds,
    DivideByZero,
    IntegerOverflow,
    InvalidConversion,
    Unreachable,
    StackOverflow,
    UndefinedElement,
    IndirectCallTypeMismatch,
    Host(String),
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trap::OutOfBounds => f.write_str("out of bounds memory access"),
            Trap::DivideByZero => f.write_str("integer divide by zero"),
            Trap::IntegerOverflow => f.write_str("integer overflow"),
            Trap::InvalidConversion => f.write_str("invalid conversion to integer"),
            Trap::Unreachable => f.write_str("unreachable executed"),
            Trap::StackOverflow => f.write_str("call stack exhausted"),
            Trap::UndefinedElement => f.write_str("undefined table element"),
            Trap::IndirectCallTypeMismatch => f.write_str("indirect call type mismatch"),
            Trap::Host(m) => write!(f, "host trap: {m}"),
        }
    }
}

impl std::error::Error for Trap {}

/// Why a fiber stopped running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    Done,
    Yield,
    OutOfBudget,
    Trap(Trap),
}
