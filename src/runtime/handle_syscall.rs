use std::io::BufRead;

use num_traits::FromPrimitive;

use crate::{funs::as_usize, runtime::data_funs::NumToStr};

use super::{ reg, syscalls::SysCalls, EAX, EBX, ECX, EDX, ESP, R1};
use efearena::Arena;

pub(super)  fn handle_syscalls(stack:&mut Arena){
    let eax = reg!(EAX);
    let syscall = SysCalls::from_usize(eax).unwrap();
    match syscall {
        SysCalls::Printchar => {
            let char = reg!(EBX);
            print!("{}",char as u8 as char); 
        },
        SysCalls::Print => {
            let start = reg!(EBX);
            let len = reg!(ECX);
            let str = stack.read(start, len);
            let str = String::from_utf8(str).unwrap();
            print!("{} ",str);
        },
        SysCalls::Println => {
            let start = reg!(EBX);
            let len = reg!(ECX);
            let str = stack.read(start, len);
            let str = String::from_utf8(str).unwrap();
            println!("{}",str);
        },
        SysCalls::Finish => {
            let code = reg!(EBX);
            std::process::exit(code as i32);
        },
        SysCalls::NumToString => {
            let r = reg!(EBX);
            let i :NumToStr;
            if reg!(ECX) == 1{
                i = NumToStr::I(r as isize);
            }else {
                i = NumToStr::U(r as usize);
            }
            let s = match reg!(EDX) {
                1 => match i {NumToStr::I(s) => format!("{:x}",s), NumToStr::U(u) => format!("{:x}",u)}
                2 => match i {NumToStr::I(s) => format!("{:b}",s), NumToStr::U(u) => format!("{:b}",u)} ,
                3 => match i {NumToStr::I(s) => format!("{:o}",s), NumToStr::U(u) => format!("{:o}",u)} ,
                _ => match i {NumToStr::I(s) => format!("{}",s), NumToStr::U(u) => format!("{}",u)} 
            };
            let s:Vec<u8> = s.as_bytes().iter().map(|a| *a).collect();
            let len = s.len();
            reg!(ECX = len);
            reg!(EBX = ESP);
            stack.write(reg!(ESP), s);
            reg!(ESP = ESP + len)
        },
        SysCalls::StringToNum => {
            let ptr = reg!(EBX);
            let len = reg!(ECX);
            let str = stack.read(ptr, len) ;//info_program!(asv STACK ptr,len);
            let str = String::from_utf8(str).unwrap();
            let num = as_usize(str).unwrap();
            reg!(EAX = num);           
        },
        SysCalls::Readln => {
            

            #[cfg(target_os = "windows")]
            let str: String = text_io::read!("{}\r");
            #[cfg(target_os = "linux")]
            let str: String = text_io::read!("{}\n");
            reg!(EBX = ESP);
            reg!(ECX = str.len());
            let s:Vec<u8> = str.as_bytes().iter().map(|a| *a).collect();
            stack.write(reg!(ESP), s);
            reg!(ESP = ESP+str.len())
        },
        SysCalls::ReadCon => {
            
            let brk = reg!(EDX) as u8;

            let mut a = Vec::new();
            std::io::stdin().lock().read_until(brk, &mut  a).unwrap();
            let str = String::from_utf8(a).unwrap();

            reg!(EBX = ESP);
            reg!(ECX = str.len());
            let s:Vec<u8> = str.as_bytes().iter().map(|a| *a).collect();
            stack.write(reg!(ESP), s);
            reg!(ESP = ESP+str.len())
        },
        SysCalls::RaylibEx1 => {
            let title = reg!(EBX);
            let len = reg!(ECX);
            let str = stack.read(title, len);

            let str = String::from_utf8(str).unwrap();
            todo!()
        },


        SysCalls::ReadFs => {
            let p = reg!(EBX);
            let len = reg!(ECX);
            let bytes = stack.read(p, len);
            let pstr = String::from_utf8(bytes).unwrap();
            let pth = std::path::Path::new(&pstr);
            let d = std::fs::read(pth).unwrap();
            let len = d.len();
            let esp = reg!(ESP);
            stack.write(esp, d);
            reg!(ESP = esp + len);
        },
        SysCalls::WriteFs => {
            let (p,pl) = (reg!(EBX),reg!(ECX));
            let to_write = stack.read(p, pl);
            // get path string
            let pstr = {
                let (t,tl) = (reg!(EDX),reg!(R1));
                let bytes = stack.read(t, tl);
                let pstr = String::from_utf8(bytes).unwrap();
                pstr
            };
            let pth = std::path::Path::new(&pstr);
            std::fs::write(pth, to_write).unwrap();
        },
    }
}
