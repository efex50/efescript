/* efe_guest.h: guest runtime for efescript mods written in C or C++ (roadmap E7.4a).
 *
 * The C counterpart of the `efe-guest` Rust crate. Header only; build the mod with
 *
 *   clang --target=wasm32 -nostdlib -O2 -Wl,--no-entry -Wl,--export=efe_scratch mod.c -o mod.wasm
 *
 * (`-Wl,--export=efe_scratch` only if the mod uses EFE_SCRATCH). The wasm ABI and the
 * `efe.provide` custom-section format are in crates/efescript/docs/e7.4-wasm-design.md.
 *
 * What it gives you:
 *   EFE_PROVIDE(name, "doc", RESULT, EFE_P(TYPE, "param"), ...)   then the function definition
 *   EFE_PROVIDE_EVENT(name, "doc") / EFE_PROVIDE_EVENT_PAYLOAD(name, "doc", "Struct")
 *   EFE_SCRATCH(size)                    the scratch region for str/bytes/struct arguments
 *   EFE_TRAP()                           trap (the `unreachable` instruction)
 *   efe_alloc / efe_free                 a bump allocator (define EFE_GUEST_IMPLEMENTATION in one
 *                                        translation unit to get it; or use wasi-libc's malloc)
 *
 * EFE_PROVIDE records need C (not C++) because they initialise char arrays from string
 * literals without the terminating NUL; in a C++ mod put the provides in a .c file or list
 * them in mod.toml (see the design doc, "Fallbacks"). Names and param names must be shorter
 * than 128 bytes, docs shorter than 16384; at most 8 parameters.
 *
 * Types in EFE_P and RESULT: I32 I64 F32 F64 BOOL ENTITY STR BYTES, and for a struct
 * parameter EFE_PS("name", "StructName"). RESULT is NONE or one of the scalar types.
 * A `str` parameter is two C parameters (const char *p, size_t n), `bytes` likewise,
 * a struct is a `const efe_Struct *`, entity is `efe_entity`, bool is `bool`.
 */
#ifndef EFE_GUEST_H
#define EFE_GUEST_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define EFE_TRAP() __builtin_trap()

#define EFE_CAT_(a, b) a##b
#define EFE_CAT(a, b) EFE_CAT_(a, b)

/* Number of arguments, 0 to 8 (GNU `##__VA_ARGS__`, as clang and gcc accept). */
#define EFE_NARGS_(z, _1, _2, _3, _4, _5, _6, _7, _8, N, ...) N
#define EFE_NARGS(...) EFE_NARGS_(0, ##__VA_ARGS__, 8, 7, 6, 5, 4, 3, 2, 1, 0)

/* One parameter is a parenthesised tuple `(TYPE, "name"...)`, made by EFE_P / EFE_PS. */
#define EFE_P(ty, nm) (ty, nm)
#define EFE_PS(nm, sn) (STRUCT, nm, sn)
#define EFE_UNP(...) __VA_ARGS__
#define EFE_X1(m, i, ...) EFE_X2(m, i, __VA_ARGS__)
#define EFE_X2(m, i, ty, ...) EFE_CAT(EFE_CAT(EFE_P_, m), EFE_CAT(_, ty))(i, __VA_ARGS__)
#define EFE_X(m, i, tuple) EFE_X1(m, i, EFE_UNP tuple)

#define EFE_FE(m, ...) EFE_CAT(EFE_FE_, EFE_NARGS(__VA_ARGS__))(m, ##__VA_ARGS__)
#define EFE_FE_0(m)
#define EFE_FE_1(m, a) EFE_X(m, 0, a)
#define EFE_FE_2(m, a, b) EFE_FE_1(m, a) EFE_X(m, 1, b)
#define EFE_FE_3(m, a, b, c) EFE_FE_2(m, a, b) EFE_X(m, 2, c)
#define EFE_FE_4(m, a, b, c, d) EFE_FE_3(m, a, b, c) EFE_X(m, 3, d)
#define EFE_FE_5(m, a, b, c, d, e) EFE_FE_4(m, a, b, c, d) EFE_X(m, 4, e)
#define EFE_FE_6(m, a, b, c, d, e, f) EFE_FE_5(m, a, b, c, d, e) EFE_X(m, 5, f)
#define EFE_FE_7(m, a, b, c, d, e, f, g) EFE_FE_6(m, a, b, c, d, e, f) EFE_X(m, 6, g)
#define EFE_FE_8(m, a, b, c, d, e, f, g, h) EFE_FE_7(m, a, b, c, d, e, f, g) EFE_X(m, 7, h)

/* A scalar parameter record: name length, name, type code. */
#define EFE_PDECL(i, nm) \
    struct { uint8_t nl; char n[sizeof(nm) - 1]; uint8_t t; } EFE_CAT(p, i);
#define EFE_PINIT(nm, code) { sizeof(nm) - 1, nm, code },
#define EFE_P_DECL_I32(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_I64(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_F32(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_F64(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_BOOL(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_ENTITY(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_STR(i, nm) EFE_PDECL(i, nm)
#define EFE_P_DECL_BYTES(i, nm) EFE_PDECL(i, nm)
#define EFE_P_INIT_I32(i, nm) EFE_PINIT(nm, 0)
#define EFE_P_INIT_I64(i, nm) EFE_PINIT(nm, 1)
#define EFE_P_INIT_F32(i, nm) EFE_PINIT(nm, 2)
#define EFE_P_INIT_F64(i, nm) EFE_PINIT(nm, 3)
#define EFE_P_INIT_BOOL(i, nm) EFE_PINIT(nm, 4)
#define EFE_P_INIT_ENTITY(i, nm) EFE_PINIT(nm, 5)
#define EFE_P_INIT_STR(i, nm) EFE_PINIT(nm, 6)
#define EFE_P_INIT_BYTES(i, nm) EFE_PINIT(nm, 7)
/* A struct parameter: the type code 8 is followed by the struct name. */
#define EFE_P_DECL_STRUCT(i, nm, sn) \
    struct { uint8_t nl; char n[sizeof(nm) - 1]; uint8_t t; uint8_t sl; char s[sizeof(sn) - 1]; } EFE_CAT(p, i);
#define EFE_P_INIT_STRUCT(i, nm, sn) { sizeof(nm) - 1, nm, 8, sizeof(sn) - 1, sn },

/* The result: NONE, or one scalar named `result`. */
#define EFE_R_DECL_NONE uint8_t nr;
#define EFE_R_INIT_NONE 0
#define EFE_RDECL uint8_t nr; struct { uint8_t nl; char n[6]; uint8_t t; } r;
#define EFE_RINIT(code) 1, { 6, "result", code }
#define EFE_R_DECL_I32 EFE_RDECL
#define EFE_R_DECL_I64 EFE_RDECL
#define EFE_R_DECL_F32 EFE_RDECL
#define EFE_R_DECL_F64 EFE_RDECL
#define EFE_R_DECL_BOOL EFE_RDECL
#define EFE_R_DECL_ENTITY EFE_RDECL
#define EFE_R_INIT_I32 EFE_RINIT(0)
#define EFE_R_INIT_I64 EFE_RINIT(1)
#define EFE_R_INIT_F32 EFE_RINIT(2)
#define EFE_R_INIT_F64 EFE_RINIT(3)
#define EFE_R_INIT_BOOL EFE_RINIT(4)
#define EFE_R_INIT_ENTITY EFE_RINIT(5)

/* A 2-byte padded uleb128 (valid LEB128, 0..16383). */
#define EFE_ULEB2(v) { 0x80 | ((v) & 0x7f), ((v) >> 7) }

#define EFE_SECTION_PROVIDE __attribute__((section(".custom_section.efe.provide"), used, aligned(1)))
#define EFE_EXPORT_AS(sym) __attribute__((export_name(sym)))

/* Exports the function that follows as `efe_provide_<fname>` and records it. */
#define EFE_PROVIDE(fname, docstr, ret, ...)                                                   \
    static const struct __attribute__((packed)) {                                              \
        uint8_t kind; uint8_t bl[2];                                                           \
        uint8_t nl; char name[sizeof(#fname) - 1];                                             \
        uint8_t el; char exp[sizeof("efe_provide_" #fname) - 1];                               \
        uint8_t dl[2]; char doc[sizeof(docstr) - 1];                                           \
        uint8_t np;                                                                            \
        EFE_FE(DECL, ##__VA_ARGS__)                                                            \
        EFE_CAT(EFE_R_DECL_, ret)                                                              \
    } EFE_CAT(efe_rec_, fname) EFE_SECTION_PROVIDE = {                                         \
        0, EFE_ULEB2(sizeof(EFE_CAT(efe_rec_, fname)) - 3),                                    \
        sizeof(#fname) - 1, #fname,                                                            \
        sizeof("efe_provide_" #fname) - 1, "efe_provide_" #fname,                              \
        EFE_ULEB2(sizeof(docstr) - 1), docstr,                                                 \
        EFE_NARGS(__VA_ARGS__),                                                                \
        EFE_FE(INIT, ##__VA_ARGS__)                                                            \
        EFE_CAT(EFE_R_INIT_, ret)                                                              \
    };                                                                                         \
    EFE_EXPORT_AS("efe_provide_" #fname)

/* An event of this mod's namespace, without or with a payload struct (by index name). */
#define EFE_PROVIDE_EVENT(ename, docstr)                                                       \
    static const struct __attribute__((packed)) {                                              \
        uint8_t kind; uint8_t bl[2];                                                           \
        uint8_t nl; char name[sizeof(#ename) - 1];                                             \
        uint8_t pflag; uint8_t dl[2]; char doc[sizeof(docstr) - 1];                            \
    } EFE_CAT(efe_evrec_, ename) EFE_SECTION_PROVIDE = {                                       \
        1, EFE_ULEB2(sizeof(EFE_CAT(efe_evrec_, ename)) - 3),                                  \
        sizeof(#ename) - 1, #ename, 0, EFE_ULEB2(sizeof(docstr) - 1), docstr                   \
    }
#define EFE_PROVIDE_EVENT_PAYLOAD(ename, docstr, sname)                                        \
    static const struct __attribute__((packed)) {                                              \
        uint8_t kind; uint8_t bl[2];                                                           \
        uint8_t nl; char name[sizeof(#ename) - 1];                                             \
        uint8_t pflag; uint8_t sl; char s[sizeof(sname) - 1];                                  \
        uint8_t dl[2]; char doc[sizeof(docstr) - 1];                                           \
    } EFE_CAT(efe_evrec_, ename) EFE_SECTION_PROVIDE = {                                       \
        1, EFE_ULEB2(sizeof(EFE_CAT(efe_evrec_, ename)) - 3),                                  \
        sizeof(#ename) - 1, #ename, 1, sizeof(sname) - 1, sname,                               \
        EFE_ULEB2(sizeof(docstr) - 1), docstr                                                  \
    }

/* The scratch region (section 13 `scratch`): the host copies str, bytes and struct arguments
 * of provided functions into `efe_scratch` before a call. One per mod; size 1..=16383.
 * Link with -Wl,--export=efe_scratch so the translator can find its address. */
#define EFE_SCRATCH(size)                                                                      \
    static const struct __attribute__((packed)) { uint8_t kind; uint8_t bl[2]; uint8_t sz[2]; } \
        efe_scratch_record EFE_SECTION_PROVIDE = { 2, EFE_ULEB2(2), EFE_ULEB2(size) };         \
    __attribute__((used, aligned(8))) uint8_t efe_scratch[size]

void *efe_alloc(size_t n);
void efe_free(void *p);

#ifdef EFE_GUEST_IMPLEMENTATION
/* A bump allocator over memory.grow; efe_free does nothing. Fine for a mod that allocates
 * little per call; link wasi-libc instead if you need a real malloc. */
#ifdef __wasm__
extern unsigned char __heap_base;
static uintptr_t efe_heap_cur, efe_heap_end;

void *efe_alloc(size_t n) {
    if (efe_heap_cur == 0) {
        efe_heap_cur = (uintptr_t)&__heap_base;
        efe_heap_end = (uintptr_t)__builtin_wasm_memory_size(0) * 65536u;
    }
    n = (n + 15u) & ~(size_t)15u;
    efe_heap_cur = (efe_heap_cur + 15u) & ~(uintptr_t)15u;
    if (efe_heap_cur + n > efe_heap_end) {
        size_t pages = (efe_heap_cur + n - efe_heap_end + 65535u) / 65536u;
        size_t old = __builtin_wasm_memory_grow(0, pages);
        if (old == (size_t)-1) return 0;
        if ((uintptr_t)old * 65536u != efe_heap_end) efe_heap_cur = (uintptr_t)old * 65536u;
        efe_heap_end = ((uintptr_t)old + pages) * 65536u;
    }
    void *p = (void *)efe_heap_cur;
    efe_heap_cur += n;
    return p;
}

void efe_free(void *p) { (void)p; }
#endif /* __wasm__ */
#endif /* EFE_GUEST_IMPLEMENTATION */

#endif /* EFE_GUEST_H */
