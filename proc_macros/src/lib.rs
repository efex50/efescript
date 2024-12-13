extern crate proc_macro;
use proc_macro::TokenStream;


/// max addr size = 2 ^ 48 -> 281474976710656
#[proc_macro]
pub fn max_addr(_i:TokenStream) -> TokenStream{
    // let a = env!("efearena_size");
    let a = 2_usize.pow(48);
    let s = format!("pub const MAX_ADDR_SIZE:usize = {};",a);
    s.parse().unwrap()
}