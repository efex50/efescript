use num_derive::{FromPrimitive, ToPrimitive};


#[repr(usize)]
/// programming apis
#[derive(Debug, PartialEq, Eq, FromPrimitive,ToPrimitive)]
pub enum SysCalls{
    /// takes:
    /// 
    /// ebx -> char
    /// 
    /// prints the char as utf8 encoded 
    /// 
    /// 4 bytes on 32 bit systens 8 on 64 bit
    Printchar = 0,

    /// rust style print!
    /// 
    /// takes:
    /// 
    /// ebx -> pointer
    /// 
    /// ecx -> lenght of string
    /// 
    Print,

    /// rust style println!
    /// 
    /// takes:
    /// 
    /// ebx -> pointer
    /// 
    /// ecx -> lenght of string
    /// 
    Println,

    /// exits the progran
    /// 
    /// takes:
    /// 
    /// ebx -> statuscode
    /// 
    /// 0 = succes 1 = fail
    Finish, 
    
    /// takes number from ebx, converts to string, then writes data to stack 
    /// 
    /// takes:
    /// 
    /// ebx -> number
    ///
    /// ecx -> 0: unsigned 1: signed default: unsigned
    /// 
    /// edx -> 0: decimal 1: hex 2: binary 3: octal default: decimal
    /// 
    /// returns:
    /// 
    /// ebx -> pointer of string
    /// 
    /// ecx -> len of str
    NumToString,

    /// takes string pointer ebx, converts to int, returns value to eax 
    /// 
    /// takes:
    /// 
    /// ebx -> pointer
    /// 
    /// ecx -> lenght of string
    /// 
    /// returns:
    /// 
    /// ebx -> number
    StringToNum,

    /// Reads console for string input max len :255
    ///
    /// writes on stack
    /// 
    /// returns:
    /// 
    /// ebx -> pointer of str
    /// 
    /// ecx -> lenght of str
    Readln,

    /// Reads console for string unitil break char from edx read input max len :usize
    ///
    /// writes on stack
    /// 
    /// returns:
    /// 
    /// ebx -> pointer of str
    /// 
    /// ecx -> lenght of str
    /// 
    /// edx -> line break u8 sign
    ReadCon,

    /// Reads from file and writes to stack
    /// 
    /// takes:
    /// 
    /// ebx -> pointer of the name to the fs
    /// 
    /// eg "./file.txt", "/home/user/data/file.txt"
    /// 
    /// ecx -> lenght of the path name
    /// 
    /// returns:
    /// 
    /// eax -> pointer of the returned data
    /// 
    /// the old esp - new esp is the len of data
    ReadFs,

    /// Writes data from stack to the given path
    /// 
    /// takes:
    /// 
    /// ebx -> pointer of data
    /// 
    /// ecx -> lenght of data
    /// 
    /// edx -> pointer of the name to the fs
    /// 
    /// eg "./file.txt", "/home/user/data/file.txt"
    /// 
    /// r1 -> lenght of the path name
    WriteFs,





    /// raylib helloworld
    RaylibEx1 = 2550,

}

