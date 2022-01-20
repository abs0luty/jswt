use std::borrow::Cow;

use jswt_common::Span;
use jswt_derive::Spannable;

#[derive(Debug, PartialEq, Spannable, Clone)]
pub struct Identifier {
    pub span: Span,
    pub value: Cow<'static, str>,
}

impl Identifier {
    pub fn new(value: &'static str, span: Span) -> Self {
        Self {
            span,
            value: value.into(),
        }
    }
}
