use num_derive::{FromPrimitive, ToPrimitive};



#[derive(Debug, PartialEq, Eq, FromPrimitive,ToPrimitive)]
pub enum SysCalls{
    /// ebx -> char
    /// 
    /// prints unitill finds the null terminator
    Printchar = 0,

    /// ebx -> pointer
    /// 
    /// ecx -> lenght of string
    /// 
    /// rust style print!
    Print,

    /// rust style println!
    /// 
    /// ebx -> pointer
    /// 
    /// ecx -> lenght of string
    Println,

    /// exits the progran
    /// 
    /// ebx -> statuscode
    /// 
    /// 0 = succes 1 = fail
    Finish, 
    
    /// takes number from ebx, converts to string, then writes data to stack 
    /// 
    /// ebx -> number
    ///
    /// ecx -> 0: unsigned 1: signed default: unsigned
    /// 
    /// edx -> 0: decimal 1: hex 2: binary 3: octal default: decimal
    /// 
    /// return:
    /// 
    /// ebx -> pointer of string
    /// 
    /// ecx -> len of str
    NumToString,

    /// takes string pointer ebx, converts to int, returns value to eax 
    /// 
    /// ebx -> pointer
    /// 
    /// ecx -> lenght of string
    /// 
    /// return:
    /// 
    /// ebx -> number
    StringToNum,

    /// Reads console for string input max len :255
    ///
    /// writes on stack
    /// 
    /// return:
    /// 
    /// ebx -> pointer of str
    /// 
    /// ecx -> lenght of str
    Readln,

    /// Reads console for string input max len :255
    ///
    /// writes on stack
    /// 
    /// return:
    /// 
    /// ebx -> pointer of str
    /// 
    /// ecx -> lenght of str
    Read,








    /// raylib helloworld
    SdlEx1 = 255,

}

