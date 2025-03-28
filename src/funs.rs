


pub fn as_usize<S:Into<String>>(a:S) -> Result<usize,()>{
    let a:String = a.into();
    let num :Result<usize, std::num::ParseIntError>;
    if a.contains("0b"){
        let without_prefix = a.trim_start_matches("0b");
        num = usize::from_str_radix(without_prefix, 2);
    }
    else if a.contains("0x"){
        let without_prefix = a.trim_start_matches("0x");
        num = usize::from_str_radix(without_prefix, 16);
    }
    else if a.contains("0o"){
        let without_prefix = a.trim_start_matches("0o");
        num = usize::from_str_radix(without_prefix, 8);
    }
    else {
        num = usize::from_str_radix(&a, 10);
    };
    num
    .map(|a|Ok(a))
    .unwrap_or(Err(()))
    
}
#[allow(unused)]
pub fn as_u8<S:Into<String>>(a:S) -> Option<u8>{
    let a:String = a.into();
    let num :Result<u8, std::num::ParseIntError>;
    if a.contains("0b"){
        let without_prefix = a.trim_start_matches("0b");
        num = u8::from_str_radix(without_prefix, 2);
    }
    else if a.contains("0x"){
        let without_prefix = a.trim_start_matches("0x");
        num = u8::from_str_radix(without_prefix, 16);
    }
    else if a.contains("0o"){
        let without_prefix = a.trim_start_matches("0o");
        num = u8::from_str_radix(without_prefix, 8);
    }
    else {
        num = u8::from_str_radix(&a, 10);
    };
    num
        .map(|a|Some(a))
        .unwrap_or(None)

}
#[allow(unused)]
pub fn as_u16<S:Into<String>>(a:S) -> Option<u16>{
    let a:String = a.into();
    let num :Result<u16, std::num::ParseIntError>;
    if a.contains("0b"){
        let without_prefix = a.trim_start_matches("0b");
        num = u16::from_str_radix(without_prefix, 2);
    }
    else if a.contains("0x"){
        let without_prefix = a.trim_start_matches("0x");
        num = u16::from_str_radix(without_prefix, 16);
    }
    else if a.contains("0o"){
        let without_prefix = a.trim_start_matches("0o");
        num = u16::from_str_radix(without_prefix, 8);
    }
    else {
        num = u16::from_str_radix(&a, 10);
    };
    num
        .map(|a|Some(a))
        .unwrap_or(None)

}
#[allow(unused)]
pub fn as_u32<S:Into<String>>(a:S) -> Option<u32>{
    let a:String = a.into();
    let num :Result<u32, std::num::ParseIntError>;
    if a.contains("0b"){
        let without_prefix = a.trim_start_matches("0b");
        num = u32::from_str_radix(without_prefix, 2);
    }
    else if a.contains("0x"){
        let without_prefix = a.trim_start_matches("0x");
        num = u32::from_str_radix(without_prefix, 16);
    }
    else if a.contains("0o"){
        let without_prefix = a.trim_start_matches("0o");
        num = u32::from_str_radix(without_prefix, 8);
    }
    else {
        num = u32::from_str_radix(&a, 10);
    };
    num
        .map(|a|Some(a))
        .unwrap_or(None)

}
#[allow(unused)]
pub fn as_u64<S:Into<String>>(a:S) -> Option<u64>{
    let a:String = a.into();
    let num :Result<u64, std::num::ParseIntError>;
    if a.contains("0b"){
        let without_prefix = a.trim_start_matches("0b");
        num = u64::from_str_radix(without_prefix, 2);
    }
    else if a.contains("0x"){
        let without_prefix = a.trim_start_matches("0x");
        num = u64::from_str_radix(without_prefix, 16);
    }
    else if a.contains("0o"){
        let without_prefix = a.trim_start_matches("0o");
        num = u64::from_str_radix(without_prefix, 8);
    }
    else {
        num = u64::from_str_radix(&a, 10);
    };
    num
        .map(|a|Some(a))
        .unwrap_or(None)

}



pub fn trim_zeroes(a:Vec<u8>) -> Vec<u8>{
    let mut hit = false;
    let mut buf = Vec::new();
    for x in a{
        if x != 0{
            hit = true;
        }
        if hit{
            buf.push(x);
        }
    };
    buf
}

pub fn get_db_data(a:&mut impl Iterator<Item = u8>){
    let mut v = Vec::new();
    while let Some(d) = a.next() {
        v.push(d);
    }
    let str = String::from_utf8(v).unwrap();
    println!("{}",str);
}

