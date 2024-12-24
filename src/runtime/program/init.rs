use super::ProgramRuntime;


impl ProgramRuntime{
    pub fn new() -> Self{
        Self::default()
    }
    pub fn load_from_file<S:Into<String>>(&mut self,a:S){
        let path:String = a.into();
        let path = std::path::Path::new(&path);
        let v = std::fs::read(path).unwrap();
        self.registers.esp = v.len();
        self.program.write(0, v);
    }
    pub fn load_from_vec(&mut self,a:Vec<u8>){
        self.registers.esp = a.len();
        self.program.write(0, a);
    }

    // ?
    pub fn load_from_vec_new(&mut self){

    }

    pub fn print_program_slice(&mut self,start:usize,len:usize) -> String{
        let v = self.program.read(start, len);
        format!("{:?}",v)
    }


}