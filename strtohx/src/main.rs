use core::panic;
use std::env::args;


#[derive(Debug)]
enum KalanType {
    K8,
    K4,
    K2,
    K1
}


macro_rules! as_le_bytes {
    (usize $v:ident ) => {
        {
            let mut v:Vec<u8> = $v;

            let mut vv = [0u8;8];
            for x in 0..8{
                let a = v.pop();
                match a{
                    Some(s) => {
                        vv[x] = s;
                    },
                    None => break,
                }
            }
            vv
        }
    };
}



fn main() {
    let args:Vec<String> = args().map(|d| d).collect();
    if args.len() < 2 {
        panic!("not enough args")
    }

    for x in 1..args.len(){
        handle_part(args[x].clone());
    }




;

}

fn handle_part(arg:String){
    let str = arg.as_bytes();
    // gelen stringi alıp 
    // push64 0xxxxxxxxxx gibi formatlayıcak
    // satır satır olucak
    let mut parts = Vec::new();
    let mut counter = 0;
    let len = str.len();
    let mut kalan = len;
    loop {
        if kalan / 8 > 0{
            kalan -= 8;
            let k = ayıkla(str,8,&mut counter); 
            parts.push(k);
        }        
        else if kalan / 4 > 0{
            kalan -= 4;
            let k = ayıkla(str,4,&mut counter); 
            parts.push(k);
        }        
        else if kalan / 2 > 0{
            kalan -= 2;
            let k = ayıkla(str,2,&mut counter); 
            parts.push(k);
        }        
        else if kalan / 1 > 0{
            kalan -= 1;
            let k = ayıkla(str,1,&mut counter); 
            parts.push(k);
        }
        else{
            break;
        }        
        
    };

    let mut out = Vec::new();
    for (x,y) in parts {
        let c = as_le_bytes!(usize y);
        let s = match x {
            KalanType::K8 => format!("push64 0x{:X}", usize::from_le_bytes(c) ),
            KalanType::K4 => format!("push32 0x{:X}", usize::from_le_bytes(c) ),
            KalanType::K2 => format!("push16 0x{:X}", usize::from_le_bytes(c) ),
            KalanType::K1 => format!("push8 0x{:X}" , usize::from_le_bytes(c) ),
        };
        out.push(s);
    }
    
    println!("------\ninput str:{}\n",arg);

    for x in out {
        println!("{}",x);
    }
    println!("\ntotal len is {}",len);

}

fn ayıkla(str:&[u8],reduce:u8,ctr:&mut i32) -> (KalanType, Vec<u8>) {

    let mut v = Vec::new();
    for x in 0..reduce{
        v.push(str[*ctr as usize]);
        *ctr += 1;
    }
    match reduce {
        8 => (KalanType::K8,v),
        4 => (KalanType::K4,v),
        2 => (KalanType::K2,v),
        1 => (KalanType::K1,v),
        _ => panic!("noluyo amk")
    }
}

