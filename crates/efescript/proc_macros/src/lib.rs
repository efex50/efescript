extern crate proc_macro;
use std::env;

use proc_macro::TokenStream;


/// max addr size = 2 ^ 48 -> 281474976710656
#[proc_macro]
pub fn max_addr(_i:TokenStream) -> TokenStream{
    // let a = env!("efearena_size");
    let a = 2_usize.pow(48);
    let s = format!("pub const MAX_ADDR_SIZE:usize = {};",a);
    s.parse().unwrap()
}



/// max stack page size = PAGE_SIZE * STACK_PAGE_COUNT
#[proc_macro]
pub fn stack_page_count(_i:TokenStream) -> TokenStream{
    // let a = env!("efearena_size");
    let a = env::var("STACK_PAGE_COUNT").unwrap_or("2".to_string());
    let a = usize::from_str_radix(&a, 10)
        .map_err(|_e| {eprintln!("{:?}",_e); panic!()})
        .unwrap();
    let s = format!("pub const STACK_PAGE_COUNT:usize = {};",a);
    s.parse().unwrap()
}