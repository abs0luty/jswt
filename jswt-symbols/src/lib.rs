mod bindings;
mod symbol;

use jswt_common::Type;

pub use self::bindings::*;
pub use self::symbol::*;

#[derive(Debug, PartialEq, Clone)]
pub struct FunctionSignature {
    pub params: Vec<Type>,
    pub returns: Type,
}

#[derive(Debug, PartialEq, Clone)]
pub struct TypeSignature {
    pub ty: Type,
}
