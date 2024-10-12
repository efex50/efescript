use efescript::{nasm_efe, runtime::ProgramRuntime};






fn main() {

    let p = ProgramRuntime;
    let f = nasm_efe::parse_from_file("./test.efe");
    let path = std::path::Path::new("./test.efec");
    std :: fs :: write(path, f.clone()).unwrap();
    p.load_from_vec(f);
    
    p.start();
}


#[cfg(test)]
mod tests{

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
    fn test_loop(){
        let mut a = 0;
        loop {
            a +=1;
            println!("{}",a);
        }
    }
}