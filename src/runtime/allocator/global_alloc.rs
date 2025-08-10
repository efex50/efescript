use crate::runtime::allocator::VMAllocator;






pub struct GlobalAllocator{

}
impl VMAllocator for GlobalAllocator{
    fn allocate(&mut self,size:usize,config:&super::AllocConfig,vm:*mut efepages::page::Page) {
        let pages = unsafe {
            &mut *vm
        };

        todo!()
    }
    
    fn free(&mut self,size:usize,config:&super::AllocConfig,vm:*mut efepages::page::Page) {
        todo!()
    }
    
    fn reallocate(&mut self,size:usize,config:&super::AllocConfig,vm:*mut efepages::page::Page) {
        todo!()
    }
}

