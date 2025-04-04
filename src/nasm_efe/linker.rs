use crate::runtime::program;

use super::PreCompile;

const SIGN:&str = "efeobjfile";


pub struct Data{
    pub blob:Vec<u8>,
    pub tag:String
}


pub struct ObjectFile{
    pub exported_funs:Vec<String>,
    pub external_funs:Vec<String>,
    pub data:Vec<Data>,
    pub program:Vec<PreCompile>,
}

impl ObjectFile {
    pub fn serialize(&self) -> Vec<u8>{
        let mut out = Vec::new();
        out.push(SIGN.bytes().collect::<Vec<u8>>());
        
        
        out.push(vec![b'\n']);
        out.push("exports:".bytes().collect::<Vec<u8>>());
        // add exports
        let mut exp = Vec::new();
        for x in &self.exported_funs{
            exp.push(x.clone());
            exp.push(";".to_string());
        }
        exp.pop();
        out.push(exp.concat().bytes().collect::<Vec<u8>>());

        
        out.push(vec![b'\n']);
        out.push("externals:".bytes().collect::<Vec<u8>>());
        // add externals
        let mut ext = Vec::new();
        for x in &self.external_funs{
            ext.push(x.clone());
            ext.push(";".to_string());
        }
        ext.pop();
        out.push(ext.concat().bytes().collect::<Vec<u8>>());
        
        
        out.push(vec![b'\n']);
        out.push("data:".bytes().collect::<Vec<u8>>());
        // add data
        let mut data = Vec::new();
        let mut meta = Vec::new();
        let mut ctr: usize = 0;

        for x in &self.data{
            meta.append(&mut x.tag.bytes().collect::<Vec<u8>>());
            meta.push(b',');
            meta.append(&mut ctr.to_be_bytes().to_vec());
            meta.push(b';');
            data.append(&mut x.blob.clone());
            ctr += x.blob.len();
        };
        meta.push(b'\n');

        out.push(meta);
        out.push(data);




        out.push(vec![b'\n']);
        out.push("program:".bytes().collect::<Vec<u8>>());
        // add externals
        todo!();



        return out.concat();
    }
    pub fn deserialize(blob:Vec<u8>) -> Self{
        todo!()
    }
}