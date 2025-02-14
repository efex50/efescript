mod calculations;

pub(super) struct Ast {
    pub head: Header,
    pub body: Body,
}

pub(super) struct Header {}

pub(super) struct Body {}

pub(super) enum AstObj {
    AstCalc,
}

