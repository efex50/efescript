use efepages::page::Page;

pub mod global_alloc;





pub struct AllocConfig{
    align:usize,
}




pub trait VMAllocator{
    fn allocate(&mut self,size:usize,config:&AllocConfig,vm:*mut Page);
    fn free(&mut self,size:usize,config:&AllocConfig,vm:*mut Page);
    fn reallocate(&mut self,size:usize,config:&AllocConfig,vm:*mut Page);
}