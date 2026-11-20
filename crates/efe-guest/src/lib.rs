//! Guest runtime for efescript mods written in Rust (roadmap E7.4a).
//!
//! A mod crate is a `#![no_std]` `cdylib` built for `wasm32-unknown-unknown`. It uses
//! bindings generated from an `.efapi` by `efescript api-export --rust`, and this crate for
//! the rest: the [`Entity`] type, a global allocator, a trapping panic handler and the
//! [`provide!`], [`provide_event!`] and [`scratch!`] macros that describe what the mod
//! offers. The wasm-level ABI and the custom-section formats are in
//! `crates/efescript/docs/e7.4-wasm-design.md`.
//!
//! On targets other than wasm32 the crate is ordinary `std` code without the allocator and
//! the panic handler, so `cargo test --workspace` works on a development machine.

#![cfg_attr(target_arch = "wasm32", no_std)]

mod entity;
pub mod heap;
pub mod record;

pub use entity::Entity;
pub use record::{ApiTy, GuestStruct, Ret, Scalar};

#[cfg(all(target_arch = "wasm32", feature = "allocator"))]
mod wasm_alloc {
    use crate::heap::{Heap, Memory, PAGE};
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::UnsafeCell;

    struct Wasm;

    impl Memory for Wasm {
        fn grow(&mut self, pages: usize) -> *mut u8 {
            let prev = core::arch::wasm32::memory_grow(0, pages);
            if prev == usize::MAX { core::ptr::null_mut() } else { (prev * PAGE) as *mut u8 }
        }
    }

    /// The global allocator: see [`crate::heap`] for the design and its limits.
    pub struct GuestAlloc(UnsafeCell<Heap>);

    // SAFETY: wasm32-unknown-unknown has no threads.
    unsafe impl Sync for GuestAlloc {}

    unsafe impl GlobalAlloc for GuestAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { (*self.0.get()).alloc(&mut Wasm, layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { (*self.0.get()).dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOC: GuestAlloc = GuestAlloc(UnsafeCell::new(Heap::new()));
}

/// A panic traps: the `unreachable` instruction ends the call with a trap (`Trap::Unreachable`
/// in the VM). The message is dropped, there is no way to print it.
#[cfg(all(target_arch = "wasm32", feature = "panic-handler"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

/// Support for the macros; not a stable API.
#[doc(hidden)]
pub mod __private {
    /// Views `(ptr, len)` as text. Traps (panics) on invalid UTF-8.
    ///
    /// # Safety
    /// The range must be readable guest memory.
    pub unsafe fn str_from_raw<'a>(ptr: *const u8, len: usize) -> &'a str {
        match core::str::from_utf8(unsafe { bytes_from_raw(ptr, len) }) {
            Ok(s) => s,
            Err(_) => panic!("provide!: a `str` argument is not valid UTF-8"),
        }
    }

    /// Views `(ptr, len)` as bytes (an empty slice for `len == 0`, whatever `ptr` is).
    ///
    /// # Safety
    /// The range must be readable guest memory.
    pub unsafe fn bytes_from_raw<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
        if len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(ptr, len) } }
    }

    /// Views a struct pointer; traps on null or a misaligned pointer.
    ///
    /// # Safety
    /// A non-null aligned `ptr` must point to a valid `T`.
    pub unsafe fn struct_from_raw<'a, T>(ptr: *const T) -> &'a T {
        if ptr.is_null() || !(ptr as usize).is_multiple_of(core::mem::align_of::<T>()) {
            panic!("provide!: a struct argument is null or misaligned");
        }
        unsafe { &*ptr }
    }

    /// The scratch region as the host sees it (`#[no_mangle]` export `efe_scratch`).
    #[repr(C, align(8))]
    pub struct Scratch<const N: usize>(pub [u8; N]);

    /// Doc lines of a macro call.
    pub const fn doc_lines<const N: usize>(lines: [&'static str; N]) -> [&'static str; N] {
        lines
    }
}

/// Exports a Rust function as a provide of this mod (section 13 of the module).
///
/// ```ignore
/// efe_guest::provide! {
///     /// Adds two numbers, saturating.
///     fn add(a: i32, b: i32) -> i32 { a.saturating_add(b) }
/// }
/// ```
///
/// The function becomes the wasm export `efe_provide_<name>` and a record in the
/// `efe.provide` custom section (name, parameter and result types, doc). Parameter types:
///
/// | written | API type | the body sees |
/// |---|---|---|
/// | `i32 i64 f32 f64 bool` | the same | the same |
/// | `Entity` | `entity` | [`Entity`] |
/// | `str` | `str` | `&str` (traps on invalid UTF-8) |
/// | `bytes` | `bytes` | `&[u8]` |
/// | `&Name` | `&Name` | `&Name` (`Name` is a generated struct) |
///
/// The result is `()` or one of the scalar types. A doc comment is required. `ptr` cannot
/// cross a mod boundary and is not accepted. If any parameter is `str`, `bytes` or a struct,
/// the crate also needs one [`scratch!`]. The result is recorded under the name `result`, so
/// no parameter may have that name.
#[macro_export]
macro_rules! provide {
    ($(#[doc = $doc:literal])+ fn $name:ident ( $($args:tt)* ) $body:block) => {
        $crate::provide!(@p [$name] [$($doc)+] [()] $body ; [] [] [] ; $($args)* ,);
    };
    ($(#[doc = $doc:literal])+ fn $name:ident ( $($args:tt)* ) -> $ret:ty $body:block) => {
        $crate::provide!(@p [$name] [$($doc)+] [$ret] $body ; [] [] [] ; $($args)* ,);
    };
    // The parameters have been read: emit the function and its record.
    (@p [$name:ident] [$($doc:literal)+] [$ret:ty] $body:block ; [$($raw:tt)*] [$($bind:tt)*] [$($api:tt)*] ; $(,)?) => {
        const _: () = {
            const DOC: &[&str] = &[$($doc),+];
            const PARAMS: &[(&str, $crate::ApiTy)] = &[$($api)*];
            const LEN: usize = $crate::record::function_len(stringify!($name), DOC, PARAMS, <$ret as $crate::Ret>::RESULTS);
            #[cfg_attr(target_arch = "wasm32", unsafe(link_section = "efe.provide"))]
            #[used]
            static RECORD: [u8; LEN] = $crate::record::function::<LEN>(stringify!($name), DOC, PARAMS, <$ret as $crate::Ret>::RESULTS);
        };
        #[unsafe(export_name = concat!("efe_provide_", stringify!($name)))]
        pub extern "C" fn $name($($raw)*) -> $ret {
            $($bind)*
            $body
        }
    };
    (@p $n:tt $d:tt $r:tt $b:tt ; [$($raw:tt)*] [$($bind:tt)*] [$($api:tt)*] ; $p:ident : str , $($rest:tt)*) => {
        $crate::provide!(@p $n $d $r $b ;
            [$($raw)* ptr: *const u8, len: usize,]
            [$($bind)* let $p: &str = unsafe { $crate::__private::str_from_raw(ptr, len) };]
            [$($api)* (stringify!($p), $crate::ApiTy::Str),] ; $($rest)*);
    };
    (@p $n:tt $d:tt $r:tt $b:tt ; [$($raw:tt)*] [$($bind:tt)*] [$($api:tt)*] ; $p:ident : bytes , $($rest:tt)*) => {
        $crate::provide!(@p $n $d $r $b ;
            [$($raw)* ptr: *const u8, len: usize,]
            [$($bind)* let $p: &[u8] = unsafe { $crate::__private::bytes_from_raw(ptr, len) };]
            [$($api)* (stringify!($p), $crate::ApiTy::Bytes),] ; $($rest)*);
    };
    (@p $n:tt $d:tt $r:tt $b:tt ; [$($raw:tt)*] [$($bind:tt)*] [$($api:tt)*] ; $p:ident : & $s:path , $($rest:tt)*) => {
        $crate::provide!(@p $n $d $r $b ;
            [$($raw)* ptr: *const $s,]
            [$($bind)* let $p: &$s = unsafe { $crate::__private::struct_from_raw(ptr) };]
            [$($api)* (stringify!($p), $crate::ApiTy::Struct(<$s as $crate::GuestStruct>::NAME)),] ; $($rest)*);
    };
    (@p $n:tt $d:tt $r:tt $b:tt ; [$($raw:tt)*] [$($bind:tt)*] [$($api:tt)*] ; $p:ident : $t:ty , $($rest:tt)*) => {
        $crate::provide!(@p $n $d $r $b ;
            [$($raw)* $p: $t,]
            [$($bind)*]
            [$($api)* (stringify!($p), <$t as $crate::Scalar>::API),] ; $($rest)*);
    };
}

/// Declares an event this mod provides in its own namespace (section 13 `pevent`).
///
/// ```ignore
/// efe_guest::provide_event! {
///     /// A coin was picked up.
///     coin_picked: Picked   // payload: a generated struct; omit `: Picked` for no payload
/// }
/// ```
#[macro_export]
macro_rules! provide_event {
    ($(#[doc = $doc:literal])+ $name:ident) => {
        $crate::provide_event!(@e [$($doc)+] [$name] [None] [::core::option::Option::None]);
    };
    ($(#[doc = $doc:literal])+ $name:ident : $payload:path) => {
        $crate::provide_event!(@e [$($doc)+] [$name] [Some] [::core::option::Option::Some(<$payload as $crate::GuestStruct>::NAME)]);
    };
    (@e [$($doc:literal)+] [$name:ident] [$_k:tt] [$payload:expr]) => {
        const _: () = {
            const DOC: &[&str] = &[$($doc),+];
            const LEN: usize = $crate::record::event_len(stringify!($name), DOC, $payload);
            #[cfg_attr(target_arch = "wasm32", unsafe(link_section = "efe.provide"))]
            #[used]
            static RECORD: [u8; LEN] = $crate::record::event::<LEN>(stringify!($name), DOC, $payload);
        };
    };
}

/// Reserves the scratch region of `$size` bytes (section 13 `scratch`): the wasm export
/// `efe_scratch` is its address, and an `efe.provide` record gives its size. The embedder
/// copies `str`, `bytes` and struct arguments of provided functions into it before a call.
/// One per crate, needed if any [`provide!`] has such a parameter.
#[macro_export]
macro_rules! scratch {
    ($size:expr) => {
        const _: () = {
            const SIZE: usize = $size;
            const LEN: usize = $crate::record::scratch_len(SIZE);
            #[cfg_attr(target_arch = "wasm32", unsafe(link_section = "efe.provide"))]
            #[used]
            static RECORD: [u8; LEN] = $crate::record::scratch::<LEN>(SIZE);
        };
        #[unsafe(no_mangle)]
        #[allow(non_upper_case_globals)]
        pub static mut efe_scratch: $crate::__private::Scratch<{ $size }> = $crate::__private::Scratch([0; { $size }]);
    };
}
