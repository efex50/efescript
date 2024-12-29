use std::path::Path;

use crate::{prelude::parse_from_file, runtime::program::thread::PThread};

use super::ProgramRuntime;


impl ProgramRuntime{
    pub fn new() -> Self{
        Self::default()
    }

    pub fn compile_from_file<S:Into<String>>(path:S) -> Self{
        
        let v = parse_from_file(path);
        let mut slf = Self::default();

        slf.program_size = v.len();
        slf.init_program();

        slf.program.write(0, v);
        slf
    }

    
    pub fn load_from_file<S:Into<String>>(&mut self,a:S){
        let path:String = a.into();
        let path = Path::new(&path);
        let v = std::fs::read(path).unwrap();
        
        self.program_size = v.len();
        self.init_program();
        
        self.program.write(0, v);
    }


    pub fn load_from_vec(&mut self,a:Vec<u8>){
        self.program_size = a.len();

        self.init_program();


        self.program.write(0, a);
    }

    // ?
    pub fn load_from_vec_new() -> Self{
        todo!()
    }

    pub fn print_program_slice(&mut self,start:usize,len:usize) -> String{
        let v = self.program.read(start, len);
        format!("{:?}",v)
    }


    fn init_program(&mut self){


        let mut main_thread = PThread::default();

        //new
        main_thread.registers.esp = self.program_size + 2;
        main_thread.program = &mut self.program;
        main_thread.id = 0;

        main_thread.stdin = &mut self.stdin;
        main_thread.stderr = &mut self.stderr;
        main_thread.stdout = &mut self.stdout;

        self.threads.push(main_thread);

        dbg!(&self);

    }

}