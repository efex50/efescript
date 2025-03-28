use std::io::BufRead;

use num_traits::FromPrimitive;

use crate::{funs::as_usize, runtime::data_funs::NumToStr, syscalls::SysCalls};

use super::{PThread, SyscallSignal};

impl PThread{
    
    pub(super) fn handle_syscalls(&mut self) -> SyscallSignal{

        let prg = unsafe {
            &mut *self.program
        };
    
    

        let syscall = SysCalls::from_usize(self.registers.ra).unwrap();
        match syscall {
            SysCalls::Printchar => {
                let char = self.registers.rb;
                print!("{}",char as u8 as char); 
            },
            SysCalls::Print => {
                let start = self.registers.rb;
                let len = self.registers.rc;
                let str = prg.read(start, len);
                let str = String::from_utf8(str).unwrap();
                print!("{} ",str);
            },
            SysCalls::Println => {
                let start = self.registers.rb;
                let len = self.registers.rc;
                let str = prg.read(start, len);
                let str = String::from_utf8(str).unwrap();
                println!("{}",str);
            },
            SysCalls::Finish => {
                let code = self.registers.rb;
                return SyscallSignal::Finish(code as u8);
            },
            SysCalls::NumToString => {
                let r = self.registers.rb;
                let i :NumToStr;
                if self.registers.rc == 1{
                    i = NumToStr::I(r as isize);
                }else {
                    i = NumToStr::U(r as usize);
                }
                let s = match self.registers.rd {
                    1 => match i {NumToStr::I(s) => format!("{:x}",s), NumToStr::U(u) => format!("{:x}",u)}
                    2 => match i {NumToStr::I(s) => format!("{:b}",s), NumToStr::U(u) => format!("{:b}",u)} ,
                    3 => match i {NumToStr::I(s) => format!("{:o}",s), NumToStr::U(u) => format!("{:o}",u)} ,
                    _ => match i {NumToStr::I(s) => format!("{}",s), NumToStr::U(u) => format!("{}",u)} 
                };
                let s:Vec<u8> = s.as_bytes().iter().map(|a| *a).collect();
                let len = s.len();
                self.registers.rc = len;
                self.registers.rb = self.registers.rsp;
                prg.write(self.registers.rsp, s);
                self.registers.rsp += len;
            },
            SysCalls::StringToNum => {
                let ptr = self.registers.rb;
                let len = self.registers.rc;
                let str = prg.read(ptr, len) ;//info_program!(asv STACK ptr,len);
                let str = String::from_utf8(str).unwrap();
                let num = as_usize(str).unwrap();
                self.registers.ra = num;
            },
            SysCalls::Readln => {


                #[cfg(target_os = "windows")]
                let str: String = text_io::read!("{}\r");
                #[cfg(target_os = "linux")]
                let str: String = text_io::read!("{}\n");
                self.registers.rb = self.registers.rsp;
                self.registers.rc = str.len();
                let s:Vec<u8> = str.as_bytes().iter().map(|a| *a).collect();
                prg.write(self.registers.rsp, s);
                self.registers.rsp += str.len();
            },
            SysCalls::ReadCon => {

                let brk = self.registers.rd as u8;

                let mut a = Vec::new();
                std::io::stdin().lock().read_until(brk, &mut  a).unwrap();
                let str = String::from_utf8(a).unwrap();

                self.registers.rb = self.registers.rsp;
                self.registers.rc = str.len();
                let s:Vec<u8> = str.as_bytes().iter().map(|a| *a).collect();
                prg.write(self.registers.rsp, s);
                self.registers.rsp += str.len();
            },
            SysCalls::RaylibEx1 => {
                let title = self.registers.rb;
                let len = self.registers.rc;
                let str = prg.read(title, len);

                let _str = String::from_utf8(str).unwrap();
                todo!("raylib not implemented")
            },


            SysCalls::ReadFs => {
                let p = self.registers.rb;
                let len = self.registers.rc;
                let bytes = prg.read(p, len);
                let pstr = String::from_utf8(bytes).unwrap();
                let pth = std::path::Path::new(&pstr);
                let d = std::fs::read(pth).unwrap();
                let len = d.len();
                let rsp = self.registers.rsp;
                prg.write(rsp, d);
                self.registers.rsp += len;
            },
            SysCalls::WriteFs => {
                let (p,pl) = (self.registers.rb,self.registers.rc);
                let to_write = prg.read(p, pl);
                // get path string
                let pstr = {
                    let (t,tl) = (self.registers.rd,self.registers.r1);
                    let bytes = prg.read(t, tl);
                    let pstr = String::from_utf8(bytes).unwrap();
                    pstr
                };
                let pth = std::path::Path::new(&pstr);
                std::fs::write(pth, to_write).unwrap();
            },
            SysCalls::HeapAlloc => {
                todo!("TODO! creates pages untill finds the end page");
                let size = self.registers.rb;
                if size == 0{
                    panic!("Heap size cannot be zero");
                }else {
                    let pointer = usize::MAX - size;
                    let a = vec![0,0];
                    dbg!("sa");
                    prg.write(pointer, a);
                    self.registers.rb = pointer;
                }
            },
            SysCalls::Write => todo!(),
            SysCalls::Flush => todo!(),
        };
        return SyscallSignal::Ok;
    }

}


