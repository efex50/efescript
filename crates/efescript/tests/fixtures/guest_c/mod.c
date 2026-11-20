/* An example guest mod in C (roadmap E7.4a). Build (needs clang with wasm32 and wasm-ld):
 *
 *   clang --target=wasm32 -nostdlib -O2 -I../../../../efe-guest/include -I.. \
 *         -Wl,--no-entry -Wl,--export=efe_scratch mod.c -o mod.wasm
 *
 * `../guest_test.efe_api.h` is the checked-in golden output of `efescript api-export --c`.
 */
#define EFE_API_IMPLEMENTATION
#define EFE_GUEST_IMPLEMENTATION
#include "guest_test.efe_api.h"
#include "efe_guest.h"

EFE_SCRATCH(4096);

EFE_PROVIDE_EVENT_PAYLOAD(coin_picked, "A coin was picked up.", "Score");
EFE_PROVIDE_EVENT(level_restarted, "The level restarted.");

EFE_PROVIDE(add, "Adds two numbers, saturating.", I32, EFE_P(I32, "a"), EFE_P(I32, "b"))
int32_t add(int32_t a, int32_t b) {
    int64_t s = (int64_t)a + b;
    return s > INT32_MAX ? INT32_MAX : s < INT32_MIN ? INT32_MIN : (int32_t)s;
}

EFE_PROVIDE(weigh, "Sums the bytes of `data`, the length of `name`, and the score and handle it is given.", F64,
            EFE_P(STR, "name"), EFE_P(BYTES, "data"), EFE_PS("cfg", "Score"), EFE_P(ENTITY, "who"), EFE_P(BOOL, "on"))
double weigh(const char *name, size_t name_len, const uint8_t *data, size_t data_len, const efe_Score *cfg,
             efe_entity who, bool on) {
    (void)name;
    double sum = 0;
    for (size_t i = 0; i < data_len; i++) sum += data[i];
    return sum + (double)name_len + (on ? cfg->points : 0) + (double)who.bits;
}

EFE_PROVIDE(checked, "Traps when `n` is negative.", I32, EFE_P(I32, "n"))
int32_t checked(int32_t n) {
    if (n < 0) EFE_TRAP();
    return n;
}

EFE_PROVIDE(run, "Calls imports of the bindings.", I32, EFE_P(I32, "n"))
int32_t run(int32_t n) {
    efe_entity e = engine_world_spawn();
    efe_Transform t = {0};
    int32_t status = engine_world_set_transform(e, &t);
    if (engine_world_is_alive(e)) status += 1;
    game_log("hello from c", 12);
    efe_Score s = {0};
    game_score_changed_send(&s);
    status += game_score_changed_count();
    game_round_reset_send();
    status += (int32_t)game_scale(1.5);
    status += game_match(n, "x", 1, (const uint8_t *)"", 0, &s) ? 1 : 0;
    int32_t *p = efe_alloc(16);
    if (p) { p[0] = n; status += p[0]; }
    return status;
}

EFE_PROVIDE(noop, "No parameters and no result.", NONE)
void noop(void) {}
