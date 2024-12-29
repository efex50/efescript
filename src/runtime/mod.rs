pub(super)  mod data_funs;

// todo
#[allow(dead_code)]
pub mod program;

use crate::*;
use nasm_efe::{parse_register_type_to_op, OperandType, SimpleOperands,};
use crate::instruction::Instuction;
