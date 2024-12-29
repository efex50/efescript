use super::{thread::PThread, ProgramRuntime};

impl ProgramRuntime{

    /// runs the program untill end
    pub fn run_all_nonstop(&mut self){
        'main:loop {
            let threads = unsafe {
                let a :*mut Vec<PThread> = &mut self.threads;
                &mut *a
            };
            for x in threads{
                match x.tick() {
                    Some(_) => (),
                    None => break 'main,
                }
            }
        }

    } 
    /// ticks all the threads
    pub fn tick_all(&mut self) -> Vec<Option<()>> {

        let threads = unsafe {
            let a :*mut Vec<PThread> = &mut self.threads;
            &mut *a
        };
        let mut v = Vec::new();
        for x in threads{
            v.push(x.tick());
        }
        v
    } 
    /// ticks selected thread
    pub fn tick(&mut self,thread:usize) -> Option<()>{

        let threads = unsafe {
            let a :*mut Vec<PThread> = &mut self.threads;
            &mut *a
        };
        
        threads[thread].tick()
    } 
}

