
/// Kullanımı:
/// 
/// ```
/// reg!(EAX) // EAX isimli u32 değerini getirir
/// 
/// reg!(EAX = 4235) // EAX isimli register'a verilen sayıyı yazar   
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


/// 
/// ```
/// // 12. index'teki byte'ti getirir
/// mem!(STACk[12]);
/// 
/// // 12. index'ten itibaren 4 byte'ı
/// // little endian halinde u32 döndürür 
/// mem!(DWORD le STACK[12]);
/// 
/// // 12. index'e verilen sayıyı yazar
/// mem!(STACK[12] = 42);
/// 
/// // 12. index'e verilen u32 tipindeki 
/// // sayıyı little endian şeklinde yazar
/// mem!(DWORD le STACK[12] = 5214124);
/// ```
#[macro_export]
macro_rules! mem {
    // getter
    ($reg:ident [$a:expr]) => {
        {
            unsafe{$reg[$a]}
        }
    };
    // getter little endian
    (WORD  le $reg:ident [$a:expr]) => {
        {
            unsafe{
                let mut a:u16 = 0;                
                a |= {($reg[$a] as u16) << 0}  ;
                a |= {($reg[$a+1] as u16) << 8}  ;
                a
            }
        }
    };
    (DWORD le $reg:ident [$a:expr]) => {
        {
            unsafe{
                let mut a:u32 = 0;                
                a |= {($reg[$a] as u32) << 0}  ;
                a |= {($reg[$a+1] as u32) << 8}  ;
                a |= {($reg[$a+2] as u32) << 16} ;
                a |= {($reg[$a+3] as u32) << 24} ;
                a   
            }
        }
    };
    
    (QWORD le $reg:ident [$a:expr]) => {
        {
            #[cfg(target_pointer_width = "64")] 
            unsafe{
                let mut a:u64 = 0;                
                a |= {($reg[$a] as u64) << 0}  ;
                a |= {($reg[$a+1] as u64) << 8}  ;
                a |= {($reg[$a+2] as u64) << 16} ;
                a |= {($reg[$a+3] as u64) << 24} ;
                a |= {($reg[$a+4] as u64) << 32} ;
                a |= {($reg[$a+5] as u64) << 40} ;
                a |= {($reg[$a+6] as u64) << 48} ;
                a |= {($reg[$a+7] as u64) << 56} ;
                a
            }
        }
    };

    // getter big endian
    (WORD be $reg:ident [$a:expr]) => {
        {
            unsafe{
                let mut a:u16 = 0;                
                a |= {($reg[$a+1] as u16) << 0}  ;
                a |= {($reg[$a] as u16) << 8}  ;
                a
            }
        }
    };
    (DWORD be $reg:ident [$a:expr] ) => {
        {
            unsafe{
                let mut a:u32 = 0;                
                a |= {($reg[$a+3] as u32) << 0}  ;
                a |= {($reg[$a+2] as u32) << 8}  ;
                a |= {($reg[$a+1] as u32) << 16} ;
                a |= {($reg[$a] as u32) << 24} ;
                a   
            }
        }
    };
    (QWORD be $reg:ident [$a:expr] ) => {
        {
            #[cfg(target_pointer_width = "64")] 
            unsafe{
                let mut a:u64 = 0;                
                a |= {($reg[$a+7] as u64) << 0}  ;
                a |= {($reg[$a+6] as u64) << 8}  ;
                a |= {($reg[$a+5] as u64) << 16} ;
                a |= {($reg[$a+4] as u64) << 24} ;
                a |= {($reg[$a+3] as u64) << 32} ;
                a |= {($reg[$a+2] as u64) << 40} ;
                a |= {($reg[$a+1] as u64) << 48} ;
                a |= {($reg[$a] as u64) << 56} ;
                a
            }
        }
    };


    // setter
    ($reg:ident [$a:expr] = $val:expr) => {
        {unsafe{$reg[$a] = $val}}
    };
    // setter little endian
    (WORD le $reg:ident [$a:expr] = $val:expr) => {
        {unsafe{
            let val:u16 = $val;
            $reg[$a]   = (val >> 0) as u8;
            $reg[$a+1] = (val >> 8) as u8;
        }}
    };
    (DWORD le $reg:ident [$a:expr] = $val:expr) => {
        {unsafe{
            let val:u32 = $val;
            $reg[$a]   = (val >> 0) as u8;
            $reg[$a+1] = (val >> 8) as u8;
            $reg[$a+2] = (val >> 8 * 2) as u8;
            $reg[$a+3] = (val >> 8 * 3) as u8;
        }}
    };
    (QWORD le $reg:ident [$a:expr] = $val:expr) => {
        {
            #[cfg(target_pointer_width = "64")] 
            unsafe{
                let val:u64 = $val;
                $reg[$a]   = (val >> 0) as u8;
                $reg[$a+1] = (val >> 8) as u8;
                $reg[$a+2] = (val >> 8 * 2) as u8;
                $reg[$a+3] = (val >> 8 * 3) as u8;
                $reg[$a+4] = (val >> 8 * 4) as u8;
                $reg[$a+5] = (val >> 8 * 5) as u8;
                $reg[$a+6] = (val >> 8 * 6) as u8;
                $reg[$a+7] = (val >> 8 * 7) as u8;
            }
        }
    };
    // setter big endian
    (WORD be $reg:ident [$a:expr] = $val:expr) => {
        {unsafe{
            let val:u16 = $val;
            $reg[$a+1]   = (val >> 0) as u8;
            $reg[$a] = (val >> 8) as u8;
        }}
    };
    (DWORD be $reg:ident [$a:expr] = $val:expr) => {
        {unsafe{
            let val:u32 = $val;
            $reg[$a+3]   = (val >> 0) as u8;
            $reg[$a+2] = (val >> 8) as u8;
            $reg[$a+1] = (val >> 8 * 2) as u8;
            $reg[$a] = (val >> 8 * 3) as u8;
        }}
    };
    (QWORD be $reg:ident [$a:expr] = $val:expr) => {
        {
            #[cfg(target_pointer_width = "64")] 
            unsafe{
            let val:u64 = $val;
            $reg[$a+7] = (val >> 0) as u8;
            $reg[$a+6] = (val >> 8) as u8;
            $reg[$a+5] = (val >> 8 * 2) as u8;
            $reg[$a+4] = (val >> 8 * 3) as u8;
            $reg[$a+3] = (val >> 8 * 4) as u8;
            $reg[$a+2] = (val >> 8 * 5) as u8;
            $reg[$a+1] = (val >> 8 * 6) as u8;
            $reg[$a] = (val >> 8 * 7) as u8;
            }
        }
    };
}

#[macro_export]
macro_rules! write_program {
    ($start:expr,$vec:ident, $($y:expr) + ) => {
        {
            let mut counter = 0;
            $(
                let y:u8 = $y;
                unsafe{
                    $vec[($start+counter)] =$y
                }
                counter += 1;
            )+
            
        }
    };
    (v $vec:ident, $start:expr, $v:expr  ) => {
        {
            let dtw:Vec<u8> = $v;
            let mut counter = 0;
            for x in dtw {
                unsafe{
                    $vec[($start+counter)] = x;
                }
                counter += 1;
            }
            
            
        }
    };
}

/// usage
/// 
/// ```
/// //Program:[u8], start , len
/// info_program!(PROGRAM, 0 , 42)
/// ```
#[macro_export]
macro_rules! info_program {
    ($program:ident ,$start:expr , $len:expr) => {
        {
            if ($len+$start) >= STACK_SIZE{
                panic!()
            }
            unsafe{
                let c:[u8;($start+$len)] = $program[$start..($start+$len)].try_into().unwrap();
                c
            }
        }
    };
    (asv $program:ident $start:expr, $len:expr ) => {
        {
            let len:usize = $len;
            let start:usize = $start;
            let mut v:Vec<u8> = Vec::new();
            if (len+start) >= STACK_SIZE{
                panic!()
            }
            for x in start..(start+len){
                unsafe{
                    v.push($program[x])
                }
            };
            v
        }
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