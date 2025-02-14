pub(super) struct AstCalcObjTree {
    h: CalcObjHead,
}

impl AstCalcObjTree {
    pub(super) fn new(left: Box<AstCalcObj>, right: Box<AstCalcObj>) -> Self {
        let h = CalcObjHead::new(left, right);
        Self { h }
    }
}



pub(super) enum AstCalcObj {
    Sum(Box<AstCalcObj>),
    Mul(Box<AstCalcObj>),
    Divide(Box<AstCalcObj>),
    Sub(Box<AstCalcObj>),
    Int(isize),
    Unt(usize),
    Identifier(String),
    Float(f32),
}


pub(super) struct CalcObjHead{
    left:Box<AstCalcObj> ,
    right:Box<AstCalcObj>
}

impl CalcObjHead {
    pub(super) fn new(left: Box<AstCalcObj>, right: Box<AstCalcObj>) -> Self {
        Self { left, right }
    }
}
