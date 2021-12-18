mod expression;
mod iteration;
mod literal;
mod statement;
mod visitor;

pub use crate::common::Ident;
pub use expression::*;
pub use iteration::*;
pub use literal::*;
pub use statement::*;
pub use visitor::*;

pub use jswt_common::{Span, Spannable};
use jswt_derive::{FromEnumVariant, Spannable};

/// Representation of the high level AST parsed directly from the source
/// with no type inference data encoded into the tree
#[derive(Debug)]
pub struct Ast {
    pub program: Program,
}

impl Ast {
    pub fn new(program: Program) -> Self {
        Self { program }
    }
}

#[derive(Debug, PartialEq)]
pub struct Program {
    pub source_elements: SourceElements,
}

#[derive(Debug, PartialEq)]
pub struct SourceElements {
    pub source_elements: Vec<SourceElement>,
}

#[derive(Debug, PartialEq, FromEnumVariant)]
pub enum SourceElement {
    FunctionDeclaration(FunctionDeclarationElement),
    Statement(StatementElement),
}

#[derive(Debug, PartialEq, Spannable)]
pub struct FunctionDeclarationElement {
    pub span: Span,
    pub decorators: FunctionDecorators,
    pub ident: Ident,
    pub params: FormalParameterList,
    pub returns: Option<TypeAnnotation>,
    pub body: FunctionBody,
}

#[derive(Debug, PartialEq)]
pub struct FunctionDecorators {
    pub annotations: Vec<Annotation>,
    pub export: bool,
}

#[derive(Debug, PartialEq, Spannable)]
pub struct Annotation {
    pub span: Span,
    pub name: Ident,
    pub expr: Option<SingleExpression>,
}

#[derive(Debug, PartialEq)]
pub struct FormalParameterList {
    pub parameters: Vec<FormalParameterArg>,
}

#[derive(Debug, PartialEq)]
pub struct FormalParameterArg {
    pub ident: Ident,
    pub type_annotation: TypeAnnotation,
}

#[derive(Debug, PartialEq, Spannable)]
pub struct FunctionBody {
    pub span: Span,
    pub source_elements: SourceElements,
}

#[derive(Debug, PartialEq)]
pub struct StatementList {
    pub statements: Vec<StatementElement>,
}

#[derive(Debug, PartialEq)]
pub enum VariableModifier {
    Let(Span),
    Const(Span),
}

impl Spannable for VariableModifier {
    fn span(&self) -> Span {
        match self {
            VariableModifier::Let(span) => span.to_owned(),
            VariableModifier::Const(span) => span.to_owned(),
        }
    }
}

#[derive(Debug, PartialEq, FromEnumVariant)]
pub enum AssignableElement {
    Identifier(Ident),
}

impl Spannable for AssignableElement {
    fn span(&self) -> Span {
        match self {
            AssignableElement::Identifier(ident) => ident.span.to_owned(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum TypeAnnotation {
    Primary(PrimaryTypeAnnotation),
}

#[derive(Debug, PartialEq)]
pub enum PrimaryTypeAnnotation {
    Reference(Ident),
    Primitive(Primitive),
    Array(Box<PrimaryTypeAnnotation>),
}

#[derive(Debug, PartialEq)]
pub enum Primitive {
    I32,
    U32,
    F32,
    Boolean,
}
