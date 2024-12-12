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



#[cfg(test)]
mod tests{
    use std::io::BufRead;


    #[test]
    fn test_str_endian(){
        let a = "dkkşfmas asş fmasşf mas3131".to_string();
        let _v = a.bytes().map(|a| a).collect::<Vec<u8>>();
    }

    #[test]
    fn test_split(){
        let d = "safafi lasfasf+ asfasfas".to_string();
        let sp:Vec<&str> = d.split("+").collect();
        if sp.len() >1{}
        println!("{:?}",sp);
    }


    #[test]
    fn test_bytes(){
    let mut a = Vec::new();
    std::io::stdin().lock().read_until(b'i', &mut  a).unwrap();
    let str = String::from_utf8(a).unwrap();
    println!("str : {}",str); 

    }
}



fn print_help(){
    println!(
    r#"
    uygulamayı kullanmak için bir program seçin

    ör: efescript test.efe
    
    efe.c olmaz
    "#
    )
}
