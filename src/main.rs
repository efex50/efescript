use std::env::args;

use efescript::prelude::*;





fn main() 
{   


    let args:Vec<String> = args().map(|d| d).collect();
    if args.len() < 2 {
        print_help();
        std::process::exit(1);
    }

    let f = &args[1];
    let mut p = ProgramRuntime::new();

    let f = parse_from_file(f);
    p.load_from_vec(f);
    p.start();
    if let Some(x) = args.get(2) {
        match x.as_str(){
            "-d" => println!("{:?}",p),
            "-r" => println!("ram usage :{:?} bytes",p.stack.page.len() * efepages::vars::PAGE_SIZE ),
            _=>()
        }
    }
}




fn print_help(){
    println!(
    r#"
    uygulamayı kullanmak için bir program seçin

    ör: efescript test.efe
    
    efe.c olmaz
    
    -d // programın debug çıktısını gösterir
    -r // programın kullandığı hafızayı byte cinsinden gösterir

    "#
    )
}
