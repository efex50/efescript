use std::env::args;
use efescript::prelude::*;


fn main() 
{   
    let mut v: Vec<u8> = Vec::new();
    

    let args:Vec<String> = args().map(|d| d).collect();
    if args.len() < 2 {
        print_help();
        std::process::exit(1);
    }

    let f = &args[1];
    let mut p = ProgramRuntime::new();

    let f = parse_from_file(f);
    std::fs::write(std::path::Path::new("./out.efec"), &f).unwrap();
    p.load_from_vec(f);
    dbg!(&p.counter);
    p.run_all_nonstop();
    if let Some(x) = args.get(2) {
        match x.as_str(){
            "-d" => println!("{:?}",p),
            "-r" => println!("ram usage :{:?} bytes",p.program.page.len() * efepages::vars::PAGE_SIZE ),
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






























#[allow(warnings)]
mod tests{
    use std::io::{stdout, BufWriter, Write};

    #[test]
    fn _flush(){
        let mut b = BufWriter::new(stdout());
        b.write("sa".as_bytes());
        println!("sa yazıldı");
        b.write("as".as_bytes());
        println!("as yazıldı");
        println!("b flush'landı");
        write!(b,"lan");
        b.write("\n".as_bytes());
        b.flush().unwrap();
    }


    #[test]
    fn vecflush(){
        let a = vec![b'a';10];
        let mut buf = BufWriter::new(stdout());
        buf.write(&a);
        buf.flush();
    }

}
#[allow(warnings)]
mod grok{
    use std::io::{self, Write};

    #[test]
    fn grok() -> io::Result<()> {
        let mut buffer = io::BufWriter::new(io::stdout());

        // Write some data to the buffer
        writeln!(buffer, "Hello, world!")?;

        // Flush the buffer to ensure the "Hello, world!" is actually printed
        buffer.flush()?;

        Ok(())
    }
}