
/// Kullanımı:
/// 
/// ```
/// use efescript::reg;
/// static mut EAX:usize = 0;
/// reg!(EAX); // EAX isimli u32 değerini getirir
/// 
/// reg!(EAX = 4235); // EAX isimli register'a verilen sayıyı yazar   
/// ```
#[macro_export]
macro_rules! reg {
    () =>{

    };
    ($reg:ident) => {
        {unsafe{$reg}}
    };
    ($reg:ident = $val:expr) => {
        {unsafe{$reg = $val}}
    };
    (p $reg:ident) => {
        {unsafe{*$reg}}
    };
    (p $reg:ident = $val:expr) => {
        {unsafe{*$reg = $val}}
    };

}



#[macro_export]
macro_rules! as_be_bytes {
    (usize $v:ident ) => {
        {
            let mut v:Vec<u8> = $v;
            let mut vv = [0u8;8];
            for x in 0..8{
                let a = v.pop();
                match a{
                    Some(s) => {
                        vv[7-x] = s;
                    },
                    None => break,
                }
            }
            vv
        }
    };
}